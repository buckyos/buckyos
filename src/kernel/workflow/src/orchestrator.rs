use crate::analysis::TRIGGER_NODE_ID;
use crate::cache_index::{step_cache_key, CacheEntry, CacheIndex, MemoryCacheIndex};
use crate::compiler::{CompiledNode, CompiledWorkflow};
use crate::dispatcher::ThunkDispatcher;
use crate::dsl::RetryFallback;
use crate::error::{WorkflowError, WorkflowResult};
use crate::executor_adapter::{ExecutorAdapter, ExecutorRegistry};
use crate::function_registry::FunctionRegistry;
use crate::native::{EnvironmentProvider, NativeIo, StaticEnvironment, ToolEnvironment};
use crate::object_store::{deterministic_object_id, WorkflowObjectStore};
use crate::runtime::{
    EventEnvelope, HumanAction, HumanActionKind, HumanWait, MapState, NodeRunState, ParJoin,
    ParState, PendingThunk, RunStatus, WorkflowRun,
};
use crate::task_tracker::{MapShardTaskView, StepTaskView, ThunkTaskView, WorkflowTaskTracker};
use crate::types::{AwaitKind, ExecutorRef, Expr, JoinStrategy, RetryPolicy, ValueTemplate};
use buckyos_api::{
    error_codes, execution_id, TaskHumanAction, ThunkExecution, ThunkExecutionError,
    ThunkExecutionResult, ThunkExecutionStatus, ThunkObject, WorkflowStepTaskData,
};
use chrono::Utc;
use ndn_lib::ObjId;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use uuid::Uuid;

/// Default reuse window of a cached step result (TODO D7: artifacts default
/// to 30 days).
pub const DEFAULT_CACHE_TTL_SECS: i64 = 30 * 86_400;

/// Per-run creation facts that are not part of the compiled definition.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub requested_by: Option<String>,
    pub execution_epoch: u64,
    /// Server-resolved run input, exposed as `${trigger.output.*}`.
    pub trigger_output: Option<Value>,
    pub deadline_at: Option<i64>,
}

pub struct WorkflowOrchestrator<D: ?Sized, O: ?Sized, T> {
    dispatcher: Arc<D>,
    object_store: Arc<O>,
    tracker: Arc<T>,
    executor_registry: Arc<ExecutorRegistry>,
    functions: Arc<FunctionRegistry>,
    cache: Arc<dyn CacheIndex>,
    environment: Arc<dyn EnvironmentProvider>,
    io: Option<Arc<dyn NativeIo>>,
    cache_ttl_secs: i64,
}

impl<D, O, T> WorkflowOrchestrator<D, O, T>
where
    D: ThunkDispatcher + ?Sized + 'static,
    O: WorkflowObjectStore + ?Sized + 'static,
    T: WorkflowTaskTracker + 'static,
{
    pub fn new(dispatcher: Arc<D>, object_store: Arc<O>, tracker: Arc<T>) -> Self {
        Self {
            dispatcher,
            object_store,
            tracker,
            executor_registry: Arc::new(ExecutorRegistry::new()),
            functions: Arc::new(FunctionRegistry::new()),
            cache: Arc::new(MemoryCacheIndex::new()),
            environment: Arc::new(StaticEnvironment(Arc::new(ToolEnvironment::default()))),
            io: None,
            cache_ttl_secs: DEFAULT_CACHE_TTL_SECS,
        }
    }

    pub fn with_functions(mut self, functions: Arc<FunctionRegistry>) -> Self {
        self.functions = functions;
        self
    }

    pub fn with_cache(mut self, cache: Arc<dyn CacheIndex>) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_environment(mut self, environment: Arc<dyn EnvironmentProvider>) -> Self {
        self.environment = environment;
        self
    }

    pub fn with_io(mut self, io: Arc<dyn NativeIo>) -> Self {
        self.io = Some(io);
        self
    }

    pub fn with_cache_ttl(mut self, secs: i64) -> Self {
        self.cache_ttl_secs = secs;
        self
    }

    pub fn functions(&self) -> &Arc<FunctionRegistry> {
        &self.functions
    }

    pub fn environment(&self) -> Arc<ToolEnvironment> {
        self.environment.current()
    }

    pub fn tracker(&self) -> &Arc<T> {
        &self.tracker
    }

    pub fn io(&self) -> Option<&Arc<dyn NativeIo>> {
        self.io.as_ref()
    }

    /// 注入编排器侧 Apply 直执行通道。命中 registry 的 executor（一期主要是
    /// `service::` / `http::` / `appservice::`）会跳过 Thunk 调度器，由编排器
    /// 同步调用 adapter 并把结果直接回填到 `node_outputs`。
    pub fn with_executor_registry(mut self, registry: Arc<ExecutorRegistry>) -> Self {
        self.executor_registry = registry;
        self
    }

    pub async fn create_run(
        &self,
        workflow: &CompiledWorkflow,
    ) -> WorkflowResult<(WorkflowRun, Vec<EventEnvelope>)> {
        self.create_run_with_metrics(workflow, BTreeMap::new())
            .await
    }

    pub async fn create_run_with_metrics(
        &self,
        workflow: &CompiledWorkflow,
        metrics: BTreeMap<String, Value>,
    ) -> WorkflowResult<(WorkflowRun, Vec<EventEnvelope>)> {
        self.create_run_with_options(workflow, metrics, RunOptions::default())
            .await
    }

    pub async fn create_run_with_options(
        &self,
        workflow: &CompiledWorkflow,
        metrics: BTreeMap<String, Value>,
        options: RunOptions,
    ) -> WorkflowResult<(WorkflowRun, Vec<EventEnvelope>)> {
        let now = Utc::now().timestamp();
        let mut node_states = BTreeMap::new();
        for node_id in workflow.nodes.keys() {
            node_states.insert(node_id.clone(), NodeRunState::Pending);
        }
        let mut node_outputs = BTreeMap::new();
        if let Some(trigger) = options.trigger_output {
            node_outputs.insert(TRIGGER_NODE_ID.to_string(), trigger);
        }
        let deadline_at = options.deadline_at.or_else(|| {
            workflow
                .definition
                .guards
                .as_ref()
                .and_then(|g| g.max_duration.as_deref().or(g.timeout.as_deref()))
                .and_then(parse_duration_secs)
                .map(|secs| now + secs as i64)
        });

        let mut run = WorkflowRun {
            run_id: format!("run-{}", Uuid::new_v4()),
            workflow_id: workflow.workflow_id.clone(),
            workflow_name: workflow.workflow_name.clone(),
            plan_version: 1,
            status: RunStatus::Created,
            node_states,
            node_outputs,
            node_attempts: BTreeMap::new(),
            branch_iterations: BTreeMap::new(),
            pending_thunks: BTreeMap::new(),
            activated_nodes: workflow.graph.start_nodes.iter().cloned().collect(),
            metrics,
            human_waiting_nodes: BTreeSet::new(),
            map_states: BTreeMap::new(),
            par_states: BTreeMap::new(),
            seq: 0,
            output: BTreeMap::new(),
            node_errors: BTreeMap::new(),
            node_sources: BTreeMap::new(),
            definition_revision: workflow.revision.clone(),
            requested_by: options.requested_by,
            execution_epoch: options.execution_epoch,
            deadline_at,
            created_at: now,
            updated_at: now,
        };

        let mut events = Vec::new();
        events.push(self.emit_event(&mut run, "run.created", None, "engine", None));
        self.tracker.sync_run(&run).await?;
        Ok((run, events))
    }

    pub async fn tick(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let mut events = Vec::new();
        if run.is_terminal() {
            return Ok(events);
        }
        if run.status == RunStatus::Created {
            run.status = RunStatus::Running;
            events.push(self.emit_event(run, "run.started", None, "engine", None));
        }
        if let Some(deadline) = run.deadline_at {
            if Utc::now().timestamp() >= deadline {
                self.abort_run(
                    workflow,
                    run,
                    ThunkExecutionError::new(error_codes::TIMEOUT, "run deadline passed")
                        .with_details(json!({"deadline_at": deadline})),
                    &mut events,
                )
                .await?;
                self.tracker.sync_run(run).await?;
                return Ok(events);
            }
        }

        loop {
            let actionable = self.actionable_nodes(workflow, run);
            if actionable.is_empty() {
                break;
            }

            let mut progressed = false;
            for node_id in actionable {
                let compiled = workflow
                    .nodes
                    .get(&node_id)
                    .ok_or_else(|| WorkflowError::NodeNotFound(node_id.clone()))?;
                match &compiled.expr {
                    Expr::Apply { .. } => {
                        self.schedule_apply(workflow, run, compiled, &mut events)
                            .await?;
                    }
                    Expr::Match { .. } => {
                        self.advance_match(workflow, run, compiled, &mut events)
                            .await?;
                    }
                    Expr::Await { .. } => {
                        self.enter_human_wait(workflow, run, compiled, &mut events)
                            .await?;
                    }
                    Expr::Par { .. } => {
                        self.enter_par(workflow, run, compiled, &mut events).await?;
                    }
                    Expr::Map { .. } => {
                        self.enter_map(workflow, run, compiled, &mut events).await?;
                    }
                }
                progressed = true;
            }

            if !progressed {
                break;
            }
        }

        self.refresh_run_status(workflow, run, &mut events);
        self.tracker.sync_run(run).await?;
        Ok(events)
    }

    /// Applies one execution receipt. Receipts for executions the run is no
    /// longer waiting for (late, duplicate, after a terminal state) are
    /// rejected with [`WorkflowError::StaleThunkResult`] and change nothing.
    pub async fn handle_thunk_result(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        result: ThunkExecutionResult,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let mut events = Vec::new();
        if run.is_terminal() {
            return Err(WorkflowError::StaleThunkResult(result.execution_id));
        }
        let Some(pending) = run.pending_thunks.get(&result.execution_id).cloned() else {
            return Err(WorkflowError::StaleThunkResult(result.execution_id));
        };
        if pending.thunk_obj_id != result.thunk_obj_id.to_string() {
            return Err(WorkflowError::StaleThunkResult(result.execution_id));
        }
        let node_id = pending.node_id.clone();
        let compiled = workflow
            .nodes
            .get(&node_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(node_id.clone()))?;

        match result.status {
            ThunkExecutionStatus::Waiting | ThunkExecutionStatus::Dispatched => {
                events.push(self.emit_event(
                    run,
                    match result.status {
                        ThunkExecutionStatus::Waiting => "step.waiting_dispatch",
                        ThunkExecutionStatus::Dispatched => "step.dispatched",
                        _ => unreachable!(),
                    },
                    Some(node_id),
                    "engine",
                    Some(json!({
                        "execution_id": result.execution_id,
                        "task_id": result.task_id,
                        "status": result.status,
                    })),
                ));
                return Ok(events);
            }
            ThunkExecutionStatus::Success => {
                run.pending_thunks.remove(&result.execution_id);
                let output = if let Some(value) = result.result.clone() {
                    value
                } else if let Some(result_obj_id) = result.result_obj_id.clone() {
                    let result_obj_id_str = result_obj_id.to_string();
                    self.object_store
                        .get_json(&result_obj_id_str)
                        .await?
                        .ok_or_else(|| {
                            WorkflowError::ObjectStore(format!(
                                "missing result object `{}`",
                                result_obj_id_str
                            ))
                        })?
                } else {
                    Value::Null
                };

                let output_id = if let Some(result_obj_id) = result.result_obj_id.clone() {
                    result_obj_id.to_string()
                } else {
                    self.object_store.put_json("wfresult", &output).await?
                };

                if let Some(shard_index) = pending.shard_index {
                    let for_each_id = node_id.clone();
                    self.handle_shard_completed(
                        workflow,
                        run,
                        &for_each_id,
                        shard_index,
                        output,
                        result.metrics.clone(),
                        &mut events,
                    )
                    .await?;
                } else {
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::Completed);
                    run.node_outputs.insert(node_id.clone(), output.clone());
                    run.node_errors.remove(&node_id);
                    run.node_sources
                        .insert(node_id.clone(), "executed".to_string());
                    run.metrics.insert(
                        node_id.clone(),
                        json!({
                            "execution_id": result.execution_id,
                            "metrics": result.metrics,
                            "fingerprint": result.fingerprint,
                            "cacheable": result.cacheable,
                        }),
                    );
                    let mut cached = false;
                    if let Some(cache_key) = pending.cache_key.as_ref() {
                        if result.cacheable {
                            let now = Utc::now().timestamp();
                            self.cache
                                .put(
                                    cache_key,
                                    &CacheEntry {
                                        result_obj_id: output_id.clone(),
                                        artifact_refs: result.artifact_refs.clone(),
                                        expires_at: Some(now + self.cache_ttl_secs),
                                        created_at: now,
                                        fingerprint: result.fingerprint.clone(),
                                    },
                                )
                                .await?;
                            cached = true;
                        }
                    }
                    self.activate_successors(workflow, run, &node_id);
                    self.notify_par_branch_completed(workflow, run, &node_id, &mut events)
                        .await?;
                    events.push(self.emit_event(
                        run,
                        "step.completed",
                        Some(node_id.clone()),
                        "engine",
                        Some(json!({
                            "execution_id": result.execution_id,
                            "result_obj_id": output_id,
                            "source": "executed",
                            "cached": cached,
                            "cacheable": result.cacheable,
                        })),
                    ));
                    self.sync_step_basic(run, compiled).await?;
                }
            }
            ThunkExecutionStatus::Failed | ThunkExecutionStatus::Cancelled => {
                run.pending_thunks.remove(&result.execution_id);
                let error = result.error.clone().unwrap_or_else(|| {
                    ThunkExecutionError::new(
                        if result.status == ThunkExecutionStatus::Cancelled {
                            error_codes::CANCELLED
                        } else {
                            error_codes::INTERNAL
                        },
                        "execution failed without an error payload",
                    )
                });
                if let Some(shard_index) = pending.shard_index {
                    self.handle_shard_failed(
                        workflow,
                        run,
                        &node_id,
                        shard_index,
                        Some(error.to_string()),
                        &mut events,
                    )
                    .await?;
                    run.node_errors.insert(node_id.clone(), error.to_value());
                    self.refresh_run_status(workflow, run, &mut events);
                    self.tracker.sync_run(run).await?;
                    return Ok(events);
                }
                let attempt = run.node_attempts.get(&node_id).copied().unwrap_or(1);
                let allow_retry = result.status == ThunkExecutionStatus::Failed;
                self.fail_apply_node(run, compiled, attempt, error, allow_retry, &mut events)
                    .await?;
            }
        }

        self.refresh_run_status(workflow, run, &mut events);
        self.tracker.sync_run(run).await?;
        Ok(events)
    }

    /// Aborts every non-terminal node, cancels pending executions and marks
    /// the run `Failed` with `error` (deadline / caller cancel paths).
    async fn abort_run(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        error: ThunkExecutionError,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        self.cancel_pending_executions(run, None).await;
        let touched: Vec<String> = run
            .node_states
            .iter()
            .filter(|(_, state)| {
                matches!(
                    state,
                    NodeRunState::Pending
                        | NodeRunState::Ready
                        | NodeRunState::Running
                        | NodeRunState::Retrying
                        | NodeRunState::WaitingHuman
                )
            })
            .map(|(id, _)| id.clone())
            .collect();
        for node_id in &touched {
            run.node_states
                .insert(node_id.clone(), NodeRunState::Aborted);
            run.human_waiting_nodes.remove(node_id);
            run.node_errors.insert(node_id.clone(), error.to_value());
        }
        run.metrics
            .insert("run_error".to_string(), error.to_value());
        run.status = if error.code == error_codes::CANCELLED {
            RunStatus::Aborted
        } else {
            RunStatus::Failed
        };
        events.push(self.emit_event(
            run,
            if run.status == RunStatus::Aborted {
                "run.aborted"
            } else {
                "run.failed"
            },
            None,
            "engine",
            Some(json!({ "error": error })),
        ));
        for node_id in touched {
            if let Some(node) = workflow.nodes.get(&node_id) {
                self.sync_step_basic(run, node).await?;
            }
        }
        Ok(())
    }

    /// Caller-driven cancel of a whole run (RPC `cancel_run`).
    pub async fn cancel_run(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        actor: &str,
        reason: Option<String>,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        if run.is_terminal() {
            return Err(WorkflowError::RunTerminal(run.run_id.clone()));
        }
        let mut events = Vec::new();
        let error = ThunkExecutionError::new(
            error_codes::CANCELLED,
            reason.unwrap_or_else(|| format!("cancelled by {actor}")),
        )
        .with_details(json!({"actor": actor}));
        self.abort_run(workflow, run, error, &mut events).await?;
        self.tracker.sync_run(run).await?;
        Ok(events)
    }

    /// Cancels pending executions of `node_id` (or of every node when `None`).
    async fn cancel_pending_executions(&self, run: &mut WorkflowRun, node_id: Option<&str>) {
        let ids: Vec<String> = run
            .pending_thunks
            .iter()
            .filter(|(_, info)| node_id.map(|n| info.node_id == n).unwrap_or(true))
            .map(|(id, _)| id.clone())
            .collect();
        for execution_id in ids {
            let _ = self.dispatcher.cancel_thunk(&execution_id).await;
            run.pending_thunks.remove(&execution_id);
        }
    }

    /// Shared failure path of a non-shard Apply node: retry, human fallback or
    /// Failed, with the structured error kept on the run and the task tree.
    async fn fail_apply_node(
        &self,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        attempt: u32,
        error: ThunkExecutionError,
        allow_retry: bool,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let policy = retry_policy(compiled);
        let node_id = compiled.id.clone();
        let mut waiting_since = None;
        run.node_errors.insert(node_id.clone(), error.to_value());
        if allow_retry && attempt < policy.max_attempts {
            run.node_states
                .insert(node_id.clone(), NodeRunState::Pending);
            events.push(self.emit_event(
                run,
                "step.retrying",
                Some(node_id.clone()),
                "engine",
                Some(json!({ "attempt": attempt + 1, "error": error })),
            ));
        } else if policy.fallback == RetryFallback::Human {
            run.node_states
                .insert(node_id.clone(), NodeRunState::WaitingHuman);
            run.human_waiting_nodes.insert(node_id.clone());
            waiting_since = Some(Utc::now().timestamp());
            events.push(self.emit_event(
                run,
                "step.waiting_human",
                Some(node_id.clone()),
                "engine",
                Some(json!({ "reason": "retries_exhausted", "error": error })),
            ));
        } else {
            let state = if error.code == error_codes::CANCELLED {
                NodeRunState::Cancelled
            } else {
                NodeRunState::Failed
            };
            run.node_states.insert(node_id.clone(), state);
            events.push(self.emit_event(
                run,
                "step.failed",
                Some(node_id.clone()),
                "engine",
                Some(json!({ "error": error })),
            ));
        }
        self.sync_step(
            run,
            compiled,
            SyncStepOpts {
                error: Some(error.to_string()),
                waiting_since,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn handle_human_action(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        action: HumanAction,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let mut events = Vec::new();
        let compiled = workflow
            .nodes
            .get(&action.node_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(action.node_id.clone()))?;

        match action.action {
            HumanActionKind::Approve => {
                self.ensure_waiting_human(run, &action.node_id)?;
                let subject = self.subject_for_node(run, compiled)?;
                run.node_outputs.insert(
                    action.node_id.clone(),
                    json!({
                        "decision": "approved",
                        "final_subject": subject,
                    }),
                );
                run.node_states
                    .insert(action.node_id.clone(), NodeRunState::Completed);
                run.human_waiting_nodes.remove(&action.node_id);
                self.activate_successors(workflow, run, &action.node_id);
            }
            HumanActionKind::Modify => {
                self.ensure_waiting_human(run, &action.node_id)?;
                let modified = action
                    .payload
                    .as_ref()
                    .and_then(|payload| payload.get("modified_subject"))
                    .cloned()
                    .ok_or_else(|| WorkflowError::InvalidHumanAction {
                        node_id: action.node_id.clone(),
                        action: "modify".to_string(),
                    })?;
                run.node_outputs.insert(
                    action.node_id.clone(),
                    json!({
                        "decision": "approved",
                        "final_subject": modified,
                    }),
                );
                run.node_states
                    .insert(action.node_id.clone(), NodeRunState::Completed);
                run.human_waiting_nodes.remove(&action.node_id);
                self.activate_successors(workflow, run, &action.node_id);
            }
            HumanActionKind::Reject => {
                self.ensure_waiting_human(run, &action.node_id)?;
                let feedback = action
                    .payload
                    .as_ref()
                    .and_then(|payload| payload.get("feedback"))
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                run.node_outputs.insert(
                    action.node_id.clone(),
                    json!({
                        "decision": "rejected",
                        "final_subject": Value::Null,
                        "feedback": feedback,
                    }),
                );
                run.node_states
                    .insert(action.node_id.clone(), NodeRunState::Completed);
                run.human_waiting_nodes.remove(&action.node_id);
                self.activate_successors(workflow, run, &action.node_id);
            }
            HumanActionKind::Retry => {
                let current = run
                    .node_states
                    .get(&action.node_id)
                    .copied()
                    .unwrap_or(NodeRunState::Pending);
                if !matches!(current, NodeRunState::WaitingHuman | NodeRunState::Failed) {
                    return Err(WorkflowError::InvalidHumanAction {
                        node_id: action.node_id.clone(),
                        action: "retry".to_string(),
                    });
                }
                run.node_states
                    .insert(action.node_id.clone(), NodeRunState::Pending);
                run.node_outputs.remove(&action.node_id);
                run.human_waiting_nodes.remove(&action.node_id);
            }
            HumanActionKind::Skip => {
                if !compiled.skippable {
                    return Err(WorkflowError::NodeNotSkippable(action.node_id));
                }
                run.node_states
                    .insert(action.node_id.clone(), NodeRunState::Skipped);
                run.node_outputs.insert(action.node_id.clone(), Value::Null);
                run.human_waiting_nodes.remove(&action.node_id);
                self.activate_successors(workflow, run, &action.node_id);
                events.push(self.emit_event(
                    run,
                    "step.skipped",
                    Some(action.node_id.clone()),
                    action.actor.as_str(),
                    None,
                ));
            }
            HumanActionKind::Abort => {
                self.cancel_pending_executions(run, None).await;
                for state in run.node_states.values_mut() {
                    if matches!(
                        state,
                        NodeRunState::Pending
                            | NodeRunState::Ready
                            | NodeRunState::Running
                            | NodeRunState::WaitingHuman
                    ) {
                        *state = NodeRunState::Aborted;
                    }
                }
                run.status = RunStatus::Aborted;
                events.push(self.emit_event(run, "run.aborted", None, action.actor.as_str(), None));
            }
            HumanActionKind::Rollback => {
                let barrier = workflow
                    .graph
                    .downstream_from(&action.node_id)
                    .into_iter()
                    .find(|node_id| {
                        workflow
                            .nodes
                            .get(node_id)
                            .map(|node| !node.idempotent)
                            .unwrap_or(false)
                            && run.node_states.get(node_id) == Some(&NodeRunState::Completed)
                    });
                if let Some(node_id) = barrier {
                    return Err(WorkflowError::RollbackBlocked(node_id));
                }
                let mut reset_nodes = workflow.graph.downstream_from(&action.node_id);
                reset_nodes.push(action.node_id.clone());
                for node_id in reset_nodes {
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::Pending);
                    run.node_outputs.remove(&node_id);
                    run.human_waiting_nodes.remove(&node_id);
                }
                run.activated_nodes.insert(action.node_id.clone());
            }
        }

        events.push(self.emit_event(
            run,
            "human.action",
            Some(action.node_id.clone()),
            action.actor.as_str(),
            Some(json!({ "action": action.action.as_str(), "payload": action.payload })),
        ));
        // 把受影响的节点（以及 rollback 时的下游链）同步到 task_manager。
        if let Some(node) = workflow.nodes.get(&action.node_id) {
            self.sync_step_basic(run, node).await?;
        }
        if matches!(action.action, HumanActionKind::Rollback) {
            for node_id in workflow.graph.downstream_from(&action.node_id) {
                if let Some(node) = workflow.nodes.get(&node_id) {
                    self.sync_step_basic(run, node).await?;
                }
            }
        }
        if matches!(action.action, HumanActionKind::Abort) {
            // Abort 触及所有未终止节点。
            let touched: Vec<String> = run
                .node_states
                .iter()
                .filter(|(_, state)| matches!(state, NodeRunState::Aborted))
                .map(|(id, _)| id.clone())
                .collect();
            for node_id in touched {
                if let Some(node) = workflow.nodes.get(&node_id) {
                    self.sync_step_basic(run, node).await?;
                }
            }
        }
        self.refresh_run_status(workflow, run, &mut events);
        self.tracker.sync_run(run).await?;
        Ok(events)
    }

    fn actionable_nodes(&self, workflow: &CompiledWorkflow, run: &mut WorkflowRun) -> Vec<String> {
        let mut result = Vec::new();
        for node_id in &run.activated_nodes {
            let state = run
                .node_states
                .get(node_id)
                .copied()
                .unwrap_or(NodeRunState::Pending);
            if state != NodeRunState::Pending {
                continue;
            }
            let dependencies = workflow.graph.dependencies(node_id);
            if dependencies
                .iter()
                .all(|dep| dependency_satisfied(workflow, run, node_id, dep))
            {
                run.node_states.insert(node_id.clone(), NodeRunState::Ready);
                result.push(node_id.clone());
            }
        }
        result
    }

    async fn schedule_apply(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let resolved_input = self.resolve_apply_input(run, compiled)?;

        // Orchestrator-side direct channel (service:: / http:: / appservice::).
        if let Some(adapter) = self.adapter_for(compiled) {
            if compiled.idempotent {
                if let Some(cached) = self.lookup_adapter_cache(compiled, &resolved_input).await? {
                    self.complete_from_cache(workflow, run, compiled, cached, events)
                        .await?;
                    return Ok(());
                }
            }
            let next_attempt = bump_attempt(run, &compiled.id);
            return self
                .schedule_apply_direct(
                    workflow,
                    run,
                    compiled,
                    resolved_input,
                    next_attempt,
                    adapter,
                    events,
                )
                .await;
        }

        // FunctionObject / thunk channel.
        let (thunk, thunk_obj_id, canonical) = self.build_thunk(compiled, resolved_input)?;
        let function = self
            .functions
            .get(&thunk.fun_id.to_string())
            .ok_or_else(|| WorkflowError::UnresolvedFunction {
                node_id: compiled.id.clone(),
                executor: thunk.fun_id.to_string(),
            })?;
        let env = self.environment.current();
        let fingerprint = function
            .native
            .as_ref()
            .map(|native| native.fingerprint(&env))
            .unwrap_or_default();
        let cacheable = compiled.idempotent && function.object.is_pure;
        let cache_key = cacheable.then(|| step_cache_key(&thunk_obj_id, &fingerprint));
        if let Some(key) = cache_key.as_ref() {
            if let Some(cached) = self.lookup_step_cache(key).await? {
                run.metrics.insert(
                    compiled.id.clone(),
                    json!({ "cache_key": key, "thunk_obj_id": thunk_obj_id, "fingerprint": fingerprint }),
                );
                self.complete_from_cache(workflow, run, compiled, cached, events)
                    .await?;
                return Ok(());
            }
        }

        let next_attempt = bump_attempt(run, &compiled.id);
        let execution_id = execution_id(&run.run_id, &compiled.id, next_attempt);
        let now = Utc::now().timestamp();
        let mut deadline_at = run.deadline_at;
        for secs in [
            step_timeout_secs(compiled),
            function.registration.timeout,
        ]
        .into_iter()
        .flatten()
        {
            let candidate = now + secs as i64;
            deadline_at = Some(deadline_at.map_or(candidate, |d| d.min(candidate)));
        }
        let execution = ThunkExecution {
            execution_id: execution_id.clone(),
            thunk_obj_id: ObjId::new(&thunk_obj_id)
                .map_err(|err| WorkflowError::Serialization(err.to_string()))?,
            run_id: run.run_id.clone(),
            node_id: compiled.id.clone(),
            attempt: next_attempt,
            execution_epoch: run.execution_epoch,
            source_obj_id: run
                .node_outputs
                .get(TRIGGER_NODE_ID)
                .and_then(|t| t.get("obj_id"))
                .and_then(Value::as_str)
                .map(str::to_string),
            requested_by: run.requested_by.clone(),
            deadline_at,
            registration: Some(function.registration.clone()),
            fingerprint: fingerprint.clone(),
            cache_key: cache_key.clone(),
        };
        self.object_store
            .put_json_with_id(&thunk_obj_id, &canonical)
            .await?;
        run.pending_thunks.insert(
            execution_id.clone(),
            PendingThunk {
                execution_id: execution_id.clone(),
                node_id: compiled.id.clone(),
                thunk_obj_id: thunk_obj_id.clone(),
                attempt: next_attempt,
                shard_index: None,
                cache_key: cache_key.clone(),
            },
        );
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Running);
        events.push(self.emit_event(
            run,
            "step.started",
            Some(compiled.id.clone()),
            "engine",
            Some(json!({
                "execution_id": execution_id,
                "thunk_obj_id": thunk_obj_id,
                "fun_id": thunk.fun_id,
                "function": format!("{}@{}", function.registration.name, function.registration.version),
                "cache_key": cache_key,
                "deadline_at": deadline_at,
            })),
        ));
        self.sync_step_basic(run, compiled).await?;
        self.sync_thunk_dispatch(run, &compiled.id, &execution_id, &thunk_obj_id, next_attempt, None)
            .await?;
        if let Err(err) = self.dispatcher.schedule_thunk(&execution, &thunk).await {
            run.pending_thunks.remove(&execution_id);
            let error = match err {
                WorkflowError::Execution(error) => error,
                other => ThunkExecutionError::new(error_codes::INTERNAL, other.to_string()),
            };
            self.fail_apply_node(run, compiled, next_attempt, error, true, events)
                .await?;
        }
        Ok(())
    }

    fn build_thunk(
        &self,
        compiled: &CompiledNode,
        resolved_input: Value,
    ) -> WorkflowResult<(ThunkObject, String, Value)> {
        let Expr::Apply {
            executor, fun_id, ..
        } = &compiled.expr
        else {
            return Err(WorkflowError::UnsupportedNode(compiled.id.clone()));
        };
        let fun_id = require_function_object(&compiled.id, executor, fun_id.as_deref())?;
        let params = match resolved_input {
            Value::Object(map) => map.into_iter().collect::<BTreeMap<_, _>>(),
            other => {
                return Err(WorkflowError::Serialization(format!(
                    "resolved thunk params for node `{}` must be a JSON object, got {}",
                    compiled.id, other
                )));
            }
        };
        let thunk = ThunkObject::new(
            ObjId::new(fun_id).map_err(|err| {
                WorkflowError::Serialization(format!("invalid fun_id `{}`: {}", fun_id, err))
            })?,
            params,
        );
        let (thunk_obj_id, canonical) = thunk
            .calc_obj_id()
            .map_err(|err| WorkflowError::Serialization(err.to_string()))?;
        let canonical: Value = serde_json::from_str(&canonical)
            .map_err(|err| WorkflowError::Serialization(err.to_string()))?;
        Ok((thunk, thunk_obj_id.to_string(), canonical))
    }

    async fn complete_from_cache(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        cached: Value,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Completed);
        run.node_outputs.insert(compiled.id.clone(), cached);
        run.node_errors.remove(&compiled.id);
        run.node_sources
            .insert(compiled.id.clone(), "cache".to_string());
        self.activate_successors(workflow, run, &compiled.id);
        self.notify_par_branch_completed(workflow, run, &compiled.id, events)
            .await?;
        events.push(self.emit_event(
            run,
            "step.completed",
            Some(compiled.id.clone()),
            "engine",
            Some(json!({ "source": "cache" })),
        ));
        self.sync_step_basic(run, compiled).await
    }

    /// Validated step cache hit: the result object must be readable and every
    /// referenced artifact still available; otherwise the entry is dropped and
    /// the step recomputed.
    async fn lookup_step_cache(&self, key: &str) -> WorkflowResult<Option<Value>> {
        let Some(entry) = self.cache.get(key).await? else {
            return Ok(None);
        };
        let now = Utc::now().timestamp();
        let mut valid = entry.expires_at.map(|exp| exp > now).unwrap_or(true);
        let mut result = None;
        if valid {
            result = crate::object_store::get_verified_json(
                self.object_store.as_ref(),
                &entry.result_obj_id,
            )
            .await?;
            valid = result.is_some();
        }
        if valid {
            if let Some(io) = self.io.as_ref() {
                for artifact in &entry.artifact_refs {
                    if !io.artifact_available(artifact).await {
                        valid = false;
                        break;
                    }
                }
            } else {
                for artifact in &entry.artifact_refs {
                    if !self.object_store.exists(artifact).await? {
                        valid = false;
                        break;
                    }
                }
            }
        }
        if !valid {
            self.cache.remove(key).await?;
            return Ok(None);
        }
        Ok(result)
    }

    fn adapter_for(&self, compiled: &CompiledNode) -> Option<Arc<dyn ExecutorAdapter>> {
        if self.executor_registry.is_empty() {
            return None;
        }
        let Expr::Apply { executor, .. } = &compiled.expr else {
            return None;
        };
        self.executor_registry.find(executor)
    }

    async fn schedule_apply_direct(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        resolved_input: Value,
        attempt: u32,
        adapter: Arc<dyn ExecutorAdapter>,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let executor = match &compiled.expr {
            Expr::Apply { executor, .. } => executor.clone(),
            _ => return Err(WorkflowError::UnsupportedNode(compiled.id.clone())),
        };
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Running);
        events.push(self.emit_event(
            run,
            "step.started",
            Some(compiled.id.clone()),
            "engine",
            Some(json!({ "executor": executor.as_str(), "mode": "direct" })),
        ));
        self.sync_step_basic(run, compiled).await?;

        match adapter.invoke(&executor, &resolved_input).await {
            Ok(output) => {
                self.complete_apply_node_direct(
                    workflow,
                    run,
                    compiled,
                    &resolved_input,
                    output,
                    Value::Null,
                    events,
                )
                .await
            }
            Err(err) => {
                self.fail_apply_node_direct(run, compiled, attempt, err, events)
                    .await?;
                Ok(())
            }
        }
    }

    async fn complete_apply_node_direct(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        resolved_input: &Value,
        output: Value,
        metrics: Value,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let output_id = self.object_store.put_json("wfresult", &output).await?;
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Completed);
        run.node_outputs.insert(compiled.id.clone(), output.clone());
        run.node_errors.remove(&compiled.id);
        run.node_sources
            .insert(compiled.id.clone(), "direct".to_string());
        run.metrics.insert(compiled.id.clone(), metrics.clone());
        if compiled.idempotent {
            let key = adapter_cache_key(compiled, resolved_input);
            let now = Utc::now().timestamp();
            self.cache
                .put(
                    &key,
                    &CacheEntry {
                        result_obj_id: output_id.clone(),
                        artifact_refs: Vec::new(),
                        expires_at: Some(now + self.cache_ttl_secs),
                        created_at: now,
                        fingerprint: BTreeMap::new(),
                    },
                )
                .await?;
        }
        self.activate_successors(workflow, run, &compiled.id);
        self.notify_par_branch_completed(workflow, run, &compiled.id, events)
            .await?;
        events.push(self.emit_event(
            run,
            "step.completed",
            Some(compiled.id.clone()),
            "engine",
            Some(json!({ "result_obj_id": output_id, "mode": "direct" })),
        ));
        self.sync_step_basic(run, compiled).await?;
        Ok(())
    }

    async fn fail_apply_node_direct(
        &self,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        attempt: u32,
        err: WorkflowError,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let error = match err {
            WorkflowError::Execution(error) => error,
            other => ThunkExecutionError::new("executor_error", other.to_string()),
        };
        self.fail_apply_node(run, compiled, attempt, error, true, events)
            .await
    }

    async fn advance_match(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let Expr::Match {
            on,
            cases,
            max_iterations,
        } = &compiled.expr
        else {
            unreachable!();
        };

        let current_iteration = run
            .branch_iterations
            .get(&compiled.id)
            .copied()
            .unwrap_or(0);
        if current_iteration >= *max_iterations {
            run.node_states
                .insert(compiled.id.clone(), NodeRunState::WaitingHuman);
            run.human_waiting_nodes.insert(compiled.id.clone());
            events.push(self.emit_event(
                run,
                "step.waiting_human",
                Some(compiled.id.clone()),
                "engine",
                Some(json!({ "reason": "max_iterations_reached" })),
            ));
            self.sync_step(
                run,
                compiled,
                SyncStepOpts {
                    waiting_since: Some(Utc::now().timestamp()),
                    ..Default::default()
                },
            )
            .await?;
            return Ok(());
        }

        let value = resolve_reference_value(run, on)?;
        let branch_key = value
            .as_str()
            .map(|value| value.to_string())
            .unwrap_or_else(|| value.to_string());
        let Some(target_id) = cases.get(&branch_key) else {
            run.node_states
                .insert(compiled.id.clone(), NodeRunState::WaitingHuman);
            run.human_waiting_nodes.insert(compiled.id.clone());
            events.push(self.emit_event(
                run,
                "step.waiting_human",
                Some(compiled.id.clone()),
                "engine",
                Some(json!({ "reason": "unexpected_branch", "branch": branch_key })),
            ));
            self.sync_step(
                run,
                compiled,
                SyncStepOpts {
                    waiting_since: Some(Utc::now().timestamp()),
                    ..Default::default()
                },
            )
            .await?;
            return Ok(());
        };

        run.branch_iterations
            .insert(compiled.id.clone(), current_iteration + 1);
        run.node_outputs
            .insert(compiled.id.clone(), json!({ "branch": branch_key }));
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Completed);
        run.activated_nodes.insert(target_id.clone());
        for successor in workflow.graph.explicit_successors(&compiled.id) {
            run.activated_nodes.insert(successor.clone());
        }
        events.push(self.emit_event(
            run,
            "step.completed",
            Some(compiled.id.clone()),
            "engine",
            None,
        ));
        self.sync_step_basic(run, compiled).await?;
        Ok(())
    }

    async fn enter_human_wait(
        &self,
        _workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let Expr::Await { prompt, .. } = &compiled.expr else {
            unreachable!();
        };
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::WaitingHuman);
        run.human_waiting_nodes.insert(compiled.id.clone());
        let subject = self.subject_for_node(run, compiled).ok();
        let subject_obj_id = match subject.as_ref() {
            Some(value) if !value.is_null() => Some(
                self.object_store
                    .put_json("workflow_subject", value)
                    .await?,
            ),
            _ => None,
        };
        let wait = HumanWait {
            node_id: compiled.id.clone(),
            prompt: prompt.clone(),
            subject: subject.clone(),
        };
        events.push(
            self.emit_event(
                run,
                "step.waiting_human",
                Some(compiled.id.clone()),
                "engine",
                Some(
                    serde_json::to_value(wait)
                        .map_err(|err| WorkflowError::Serialization(err.to_string()))?,
                ),
            ),
        );
        let waiting_since = Utc::now().timestamp();
        self.sync_step(
            run,
            compiled,
            SyncStepOpts {
                waiting_since: Some(waiting_since),
                subject_obj_id,
                subject_value: subject,
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    fn refresh_run_status(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        events: &mut Vec<EventEnvelope>,
    ) {
        run.updated_at = Utc::now().timestamp();

        if run.status == RunStatus::Aborted {
            return;
        }

        if run
            .node_states
            .values()
            .any(|state| *state == NodeRunState::WaitingHuman)
        {
            run.status = RunStatus::WaitingHuman;
            return;
        }

        if run
            .node_states
            .values()
            .any(|state| *state == NodeRunState::Running)
        {
            run.status = RunStatus::Running;
            return;
        }

        let actionable = run
            .activated_nodes
            .iter()
            .any(|node_id| run.node_states.get(node_id) == Some(&NodeRunState::Pending));
        if actionable {
            run.status = RunStatus::Running;
            return;
        }

        if run
            .node_states
            .values()
            .any(|state| *state == NodeRunState::Failed)
        {
            if run.status != RunStatus::Failed {
                run.status = RunStatus::Failed;
                events.push(self.emit_event(run, "run.failed", None, "engine", None));
            }
            return;
        }

        if run.status != RunStatus::Completed {
            run.status = RunStatus::Completed;
            run.output = workflow
                .graph
                .terminal_from
                .iter()
                .filter_map(|node_id| {
                    run.node_outputs
                        .get(node_id)
                        .map(|value| (node_id.clone(), value.clone()))
                })
                .collect();
            events.push(self.emit_event(run, "run.completed", None, "engine", Some(json!({
                "terminal_nodes": workflow.graph.terminal_from.iter().cloned().collect::<Vec<_>>()
            }))));
        }
    }

    fn activate_successors(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        node_id: &str,
    ) {
        for successor in workflow.graph.explicit_successors(node_id) {
            run.activated_nodes.insert(successor.clone());
        }
    }

    fn resolve_apply_input(
        &self,
        run: &WorkflowRun,
        compiled: &CompiledNode,
    ) -> WorkflowResult<Value> {
        let Expr::Apply { params, .. } = &compiled.expr else {
            return Err(WorkflowError::UnsupportedNode(compiled.id.clone()));
        };
        let mut resolved = serde_json::Map::new();
        for (key, value) in params {
            resolved.insert(key.clone(), resolve_template_value(run, value)?);
        }
        Ok(Value::Object(resolved))
    }

    async fn lookup_adapter_cache(
        &self,
        compiled: &CompiledNode,
        resolved_input: &Value,
    ) -> WorkflowResult<Option<Value>> {
        let key = adapter_cache_key(compiled, resolved_input);
        self.lookup_step_cache(&key).await
    }

    fn subject_for_node(
        &self,
        run: &WorkflowRun,
        compiled: &CompiledNode,
    ) -> WorkflowResult<Value> {
        match &compiled.expr {
            Expr::Await {
                kind: AwaitKind::Confirm,
                subject: Some(subject),
                ..
            } => resolve_reference_value(run, subject),
            Expr::Await { .. } => Ok(Value::Null),
            _ => Err(WorkflowError::InvalidHumanAction {
                node_id: compiled.id.clone(),
                action: "subject".to_string(),
            }),
        }
    }

    fn ensure_waiting_human(&self, run: &WorkflowRun, node_id: &str) -> WorkflowResult<()> {
        if run.node_states.get(node_id) != Some(&NodeRunState::WaitingHuman) {
            return Err(WorkflowError::NodeNotWaitingHuman(node_id.to_string()));
        }
        Ok(())
    }

    fn emit_event(
        &self,
        run: &mut WorkflowRun,
        event_type: &str,
        node_id: Option<String>,
        actor: &str,
        payload: Option<Value>,
    ) -> EventEnvelope {
        run.seq += 1;
        EventEnvelope {
            event_id: format!("evt-{}", Uuid::new_v4()),
            event_type: event_type.to_string(),
            ts: Utc::now().to_rfc3339(),
            run_id: run.run_id.clone(),
            plan_version: run.plan_version,
            seq: run.seq,
            actor: actor.to_string(),
            node_id,
            attempt: None,
            payload,
        }
    }

    async fn enter_par(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let Expr::Par { branches, join } = &compiled.expr else {
            unreachable!();
        };
        let par_join = match join {
            JoinStrategy::All => ParJoin::All,
            JoinStrategy::Any => ParJoin::Any,
            JoinStrategy::NOfM(n) => ParJoin::NOfM(*n),
        };
        run.par_states.insert(
            compiled.id.clone(),
            ParState {
                node_id: compiled.id.clone(),
                branches: branches.clone(),
                join: par_join,
                completed_count: 0,
            },
        );
        for branch_id in branches {
            run.activated_nodes.insert(branch_id.clone());
            run.node_states
                .entry(branch_id.clone())
                .and_modify(|state| {
                    if matches!(*state, NodeRunState::Ready) {
                        *state = NodeRunState::Pending;
                    }
                })
                .or_insert(NodeRunState::Pending);
        }
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Running);
        events.push(self.emit_event(
            run,
            "step.started",
            Some(compiled.id.clone()),
            "engine",
            Some(json!({ "branches": branches })),
        ));
        self.sync_step_basic(run, compiled).await?;
        let _ = workflow;
        Ok(())
    }

    async fn notify_par_branch_completed(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        finished_node_id: &str,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let par_ids: Vec<String> = run
            .par_states
            .iter()
            .filter(|(_, state)| state.branches.iter().any(|id| id == finished_node_id))
            .map(|(id, _)| id.clone())
            .collect();

        for par_id in par_ids {
            self.check_par_completion(workflow, run, &par_id, events)
                .await?;
        }
        Ok(())
    }

    async fn check_par_completion(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        par_id: &str,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let state = run
            .par_states
            .get(par_id)
            .cloned()
            .ok_or_else(|| WorkflowError::MissingParState(par_id.to_string()))?;

        let mut completed_branches: Vec<String> = Vec::new();
        let mut any_failed = false;
        for branch_id in &state.branches {
            match run.node_states.get(branch_id) {
                Some(NodeRunState::Completed) | Some(NodeRunState::Skipped) => {
                    completed_branches.push(branch_id.clone());
                }
                Some(NodeRunState::Failed) => {
                    any_failed = true;
                }
                _ => {}
            }
        }

        let join_satisfied = match state.join {
            ParJoin::All => completed_branches.len() == state.branches.len(),
            ParJoin::Any => !completed_branches.is_empty(),
            ParJoin::NOfM(n) => completed_branches.len() as u32 >= n,
        };

        if any_failed && state.join == ParJoin::All {
            run.node_states
                .insert(par_id.to_string(), NodeRunState::Failed);
            run.par_states.remove(par_id);
            events.push(self.emit_event(
                run,
                "step.failed",
                Some(par_id.to_string()),
                "engine",
                Some(json!({ "reason": "branch_failed" })),
            ));
            if let Some(par_compiled) = workflow.nodes.get(par_id) {
                self.sync_step_basic(run, par_compiled).await?;
            }
            return Ok(());
        }

        if !join_satisfied {
            return Ok(());
        }

        let mut output_map = serde_json::Map::new();
        for branch_id in &completed_branches {
            let value = run
                .node_outputs
                .get(branch_id)
                .cloned()
                .unwrap_or(Value::Null);
            output_map.insert(branch_id.clone(), value);
        }

        let mut cancelled = Vec::new();
        for branch_id in &state.branches {
            if completed_branches.contains(branch_id) {
                continue;
            }
            if let Some(branch_state) = run.node_states.get(branch_id).copied() {
                if matches!(
                    branch_state,
                    NodeRunState::Pending
                        | NodeRunState::Ready
                        | NodeRunState::Running
                        | NodeRunState::WaitingHuman
                ) {
                    if branch_state == NodeRunState::Running {
                        self.cancel_pending_executions(run, Some(branch_id)).await;
                    }
                    run.node_states
                        .insert(branch_id.clone(), NodeRunState::Cancelled);
                    run.activated_nodes.remove(branch_id);
                    cancelled.push(branch_id.clone());
                }
            }
        }

        run.node_outputs
            .insert(par_id.to_string(), Value::Object(output_map));
        run.node_states
            .insert(par_id.to_string(), NodeRunState::Completed);
        run.par_states.remove(par_id);
        self.activate_successors(workflow, run, par_id);
        events.push(self.emit_event(
            run,
            "step.completed",
            Some(par_id.to_string()),
            "engine",
            Some(json!({
                "completed_branches": completed_branches,
                "cancelled_branches": cancelled,
            })),
        ));
        for branch_id in &cancelled {
            if let Some(branch_compiled) = workflow.nodes.get(branch_id) {
                self.sync_step_basic(run, branch_compiled).await?;
            }
        }
        if let Some(par_compiled) = workflow.nodes.get(par_id) {
            self.sync_step_basic(run, par_compiled).await?;
        }
        Ok(())
    }

    async fn enter_map(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        compiled: &CompiledNode,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let Expr::Map {
            collection,
            steps,
            max_items,
            actual_concurrency,
            ..
        } = &compiled.expr
        else {
            unreachable!();
        };
        if steps.is_empty() {
            return Err(WorkflowError::ForEachBodyNotApply(
                compiled.id.clone(),
                "<empty>".to_string(),
            ));
        }
        let body_step_id = steps[0].clone();
        let body_node = workflow
            .nodes
            .get(&body_step_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(body_step_id.clone()))?;
        if !matches!(body_node.expr, Expr::Apply { .. }) {
            return Err(WorkflowError::ForEachBodyNotApply(
                compiled.id.clone(),
                body_step_id.clone(),
            ));
        }

        let collection_value = resolve_reference_value(run, collection)?;
        let items =
            extract_items(&collection_value).ok_or_else(|| WorkflowError::ForEachItemsType {
                node_id: compiled.id.clone(),
                actual: format!("{:?}", collection_value),
            })?;

        if items.len() as u32 > *max_items {
            return Err(WorkflowError::ForEachTooManyItems {
                node_id: compiled.id.clone(),
                count: items.len() as u32,
                max: *max_items,
            });
        }

        let total = items.len();
        let map_state = MapState {
            for_each_id: compiled.id.clone(),
            body_step_id: body_step_id.clone(),
            items: items.clone(),
            shard_states: vec![NodeRunState::Pending; total],
            shard_outputs: vec![Value::Null; total],
            shard_attempts: vec![0; total],
            max_concurrency: (*actual_concurrency).max(1),
        };
        run.map_states.insert(compiled.id.clone(), map_state);
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Running);
        events.push(self.emit_event(
            run,
            "step.started",
            Some(compiled.id.clone()),
            "engine",
            Some(json!({ "total": total, "concurrency": actual_concurrency })),
        ));
        self.sync_step_basic(run, compiled).await?;

        if total == 0 {
            self.complete_map(workflow, run, &compiled.id, events)
                .await?;
            return Ok(());
        }

        self.dispatch_pending_shards(workflow, run, &compiled.id, body_node, events)
            .await?;
        Ok(())
    }

    async fn dispatch_pending_shards(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        for_each_id: &str,
        body_node: &CompiledNode,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let (max_concurrency, total, _body_step_id) = {
            let state = run
                .map_states
                .get(for_each_id)
                .ok_or_else(|| WorkflowError::MissingMapState(for_each_id.to_string()))?;
            (
                state.max_concurrency,
                state.items.len(),
                state.body_step_id.clone(),
            )
        };

        let mut running_count = run
            .map_states
            .get(for_each_id)
            .map(|state| {
                state
                    .shard_states
                    .iter()
                    .filter(|s| matches!(*s, NodeRunState::Running))
                    .count() as u32
            })
            .unwrap_or(0);

        // body 节点固定，所以 adapter 决策在所有 shard 间一致。
        let adapter = self.adapter_for(body_node);

        for index in 0..total {
            // 直执行通道下 shard 同步完成、不会占用 running_count；只有 Thunk 路径
            // 才需要受 max_concurrency 限制。
            if adapter.is_none() && running_count >= max_concurrency {
                break;
            }
            let shard_status = run
                .map_states
                .get(for_each_id)
                .and_then(|state| state.shard_states.get(index).copied());
            if shard_status != Some(NodeRunState::Pending) {
                continue;
            }
            let item = run
                .map_states
                .get(for_each_id)
                .and_then(|state| state.items.get(index).cloned())
                .unwrap_or(Value::Null);
            let attempt = {
                let state = run.map_states.get_mut(for_each_id).unwrap();
                state.shard_attempts[index] = state.shard_attempts[index] + 1;
                state.shard_attempts[index]
            };
            let resolved_input =
                self.resolve_shard_input(run, body_node, item.clone(), index as u32)?;

            if let Some(adapter) = adapter.as_ref() {
                let executor = match &body_node.expr {
                    Expr::Apply { executor, .. } => executor.clone(),
                    _ => return Err(WorkflowError::UnsupportedNode(body_node.id.clone())),
                };
                let _ = attempt;
                match adapter.invoke(&executor, &resolved_input).await {
                    Ok(output) => {
                        if let Some(state) = run.map_states.get_mut(for_each_id) {
                            state.shard_states[index] = NodeRunState::Completed;
                            state.shard_outputs[index] = output;
                        }
                        events.push(self.emit_event(
                            run,
                            "step.progress",
                            Some(for_each_id.to_string()),
                            "engine",
                            Some(json!({
                                "shard": index,
                                "status": "completed",
                                "mode": "direct",
                            })),
                        ));
                        self.sync_shard(run, for_each_id, index as u32, None)
                            .await?;
                    }
                    Err(err) => {
                        let error_message = err.to_string();
                        if let Some(state) = run.map_states.get_mut(for_each_id) {
                            state.shard_states[index] = NodeRunState::Failed;
                        }
                        run.node_states
                            .insert(for_each_id.to_string(), NodeRunState::Failed);
                        events.push(self.emit_event(
                            run,
                            "step.failed",
                            Some(for_each_id.to_string()),
                            "engine",
                            Some(json!({
                                "shard": index,
                                "error": error_message.clone(),
                                "mode": "direct",
                            })),
                        ));
                        self.sync_shard(
                            run,
                            for_each_id,
                            index as u32,
                            Some(error_message.clone()),
                        )
                        .await?;
                        if let Some(parent) = workflow.nodes.get(for_each_id) {
                            self.sync_step(
                                run,
                                parent,
                                SyncStepOpts {
                                    error: Some(error_message),
                                    ..Default::default()
                                },
                            )
                            .await?;
                        }
                        return Ok(());
                    }
                }
                continue;
            }

            let (thunk, thunk_obj_id, canonical) = self.build_thunk(body_node, resolved_input)?;
            let function = self
                .functions
                .get(&thunk.fun_id.to_string())
                .ok_or_else(|| WorkflowError::UnresolvedFunction {
                    node_id: body_node.id.clone(),
                    executor: thunk.fun_id.to_string(),
                })?;
            let shard_node_id = format!("{}[{}]", for_each_id, index);
            let execution_id = execution_id(&run.run_id, &shard_node_id, attempt);
            let env = self.environment.current();
            let fingerprint = function
                .native
                .as_ref()
                .map(|native| native.fingerprint(&env))
                .unwrap_or_default();
            let now = Utc::now().timestamp();
            let mut deadline_at = run.deadline_at;
            for secs in [step_timeout_secs(body_node), function.registration.timeout]
                .into_iter()
                .flatten()
            {
                let candidate = now + secs as i64;
                deadline_at = Some(deadline_at.map_or(candidate, |d| d.min(candidate)));
            }
            let execution = ThunkExecution {
                execution_id: execution_id.clone(),
                thunk_obj_id: ObjId::new(&thunk_obj_id)
                    .map_err(|err| WorkflowError::Serialization(err.to_string()))?,
                run_id: run.run_id.clone(),
                node_id: shard_node_id,
                attempt,
                execution_epoch: run.execution_epoch,
                source_obj_id: None,
                requested_by: run.requested_by.clone(),
                deadline_at,
                registration: Some(function.registration.clone()),
                fingerprint,
                cache_key: None,
            };
            self.object_store
                .put_json_with_id(&thunk_obj_id, &canonical)
                .await?;
            run.pending_thunks.insert(
                execution_id.clone(),
                PendingThunk {
                    execution_id: execution_id.clone(),
                    node_id: for_each_id.to_string(),
                    thunk_obj_id: thunk_obj_id.clone(),
                    attempt,
                    shard_index: Some(index as u32),
                    cache_key: None,
                },
            );
            if let Some(state) = run.map_states.get_mut(for_each_id) {
                state.shard_states[index] = NodeRunState::Running;
            }
            self.sync_shard(run, for_each_id, index as u32, None)
                .await?;
            self.sync_thunk_dispatch(
                run,
                for_each_id,
                &execution_id,
                &thunk_obj_id,
                attempt,
                Some(index as u32),
            )
            .await?;
            if let Err(err) = self.dispatcher.schedule_thunk(&execution, &thunk).await {
                run.pending_thunks.remove(&execution_id);
                let error = match err {
                    WorkflowError::Execution(error) => error,
                    other => ThunkExecutionError::new(error_codes::INTERNAL, other.to_string()),
                };
                self.handle_shard_failed(
                    workflow,
                    run,
                    for_each_id,
                    index as u32,
                    Some(error.to_string()),
                    events,
                )
                .await?;
                run.node_errors
                    .insert(for_each_id.to_string(), error.to_value());
                return Ok(());
            }
            running_count += 1;
        }

        if adapter.is_some() {
            let all_done = run
                .map_states
                .get(for_each_id)
                .map(|state| {
                    state
                        .shard_states
                        .iter()
                        .all(|s| matches!(*s, NodeRunState::Completed))
                })
                .unwrap_or(false);
            if all_done {
                self.complete_map(workflow, run, for_each_id, events)
                    .await?;
            }
        }
        Ok(())
    }

    fn resolve_shard_input(
        &self,
        run: &WorkflowRun,
        body_node: &CompiledNode,
        item: Value,
        index: u32,
    ) -> WorkflowResult<Value> {
        let Expr::Apply { params, .. } = &body_node.expr else {
            return Err(WorkflowError::UnsupportedNode(body_node.id.clone()));
        };
        let mut resolved = serde_json::Map::new();
        for (key, value) in params {
            resolved.insert(key.clone(), resolve_template_value(run, value)?);
        }
        resolved.insert("_item".to_string(), item);
        resolved.insert("_index".to_string(), Value::from(index));
        Ok(Value::Object(resolved))
    }

    async fn handle_shard_completed(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        for_each_id: &str,
        shard_index: u32,
        output: Value,
        metrics: Value,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        {
            let state = run
                .map_states
                .get_mut(for_each_id)
                .ok_or_else(|| WorkflowError::MissingMapState(for_each_id.to_string()))?;
            let idx = shard_index as usize;
            if idx >= state.shard_states.len() {
                return Err(WorkflowError::MissingMapState(for_each_id.to_string()));
            }
            state.shard_states[idx] = NodeRunState::Completed;
            state.shard_outputs[idx] = output.clone();
        }
        let _ = metrics;
        events.push(self.emit_event(
            run,
            "step.progress",
            Some(for_each_id.to_string()),
            "engine",
            Some(json!({ "shard": shard_index, "status": "completed" })),
        ));
        self.sync_shard(run, for_each_id, shard_index, None).await?;

        let body_step_id = run
            .map_states
            .get(for_each_id)
            .map(|state| state.body_step_id.clone())
            .ok_or_else(|| WorkflowError::MissingMapState(for_each_id.to_string()))?;
        let body_node = workflow
            .nodes
            .get(&body_step_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(body_step_id.clone()))?;
        self.dispatch_pending_shards(workflow, run, for_each_id, body_node, events)
            .await?;

        let all_done = run
            .map_states
            .get(for_each_id)
            .map(|state| {
                state
                    .shard_states
                    .iter()
                    .all(|s| matches!(*s, NodeRunState::Completed))
            })
            .unwrap_or(false);

        if all_done {
            self.complete_map(workflow, run, for_each_id, events)
                .await?;
        }
        Ok(())
    }

    async fn handle_shard_failed(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        for_each_id: &str,
        shard_index: u32,
        error: Option<String>,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        {
            let state = run
                .map_states
                .get_mut(for_each_id)
                .ok_or_else(|| WorkflowError::MissingMapState(for_each_id.to_string()))?;
            let idx = shard_index as usize;
            state.shard_states[idx] = NodeRunState::Failed;
        }
        events.push(self.emit_event(
            run,
            "step.failed",
            Some(for_each_id.to_string()),
            "engine",
            Some(json!({ "shard": shard_index, "error": error })),
        ));
        run.node_states
            .insert(for_each_id.to_string(), NodeRunState::Failed);
        self.sync_shard(run, for_each_id, shard_index, error.clone())
            .await?;
        if let Some(parent) = workflow.nodes.get(for_each_id) {
            self.sync_step(
                run,
                parent,
                SyncStepOpts {
                    error,
                    ..Default::default()
                },
            )
            .await?;
        }
        Ok(())
    }

    async fn complete_map(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        for_each_id: &str,
        events: &mut Vec<EventEnvelope>,
    ) -> WorkflowResult<()> {
        let outputs = run
            .map_states
            .get(for_each_id)
            .map(|state| state.shard_outputs.clone())
            .unwrap_or_default();
        run.node_outputs
            .insert(for_each_id.to_string(), Value::Array(outputs));
        run.node_states
            .insert(for_each_id.to_string(), NodeRunState::Completed);
        run.map_states.remove(for_each_id);
        self.activate_successors(workflow, run, for_each_id);
        self.notify_par_branch_completed(workflow, run, for_each_id, events)
            .await?;
        events.push(self.emit_event(
            run,
            "step.completed",
            Some(for_each_id.to_string()),
            "engine",
            None,
        ));
        if let Some(parent) = workflow.nodes.get(for_each_id) {
            self.sync_step_basic(run, parent).await?;
        }
        Ok(())
    }

    /// 把 compiled 节点的当前状态同步到 task_manager 上对应的 Step task。
    /// 默认情况下从 run 里读 state / attempt / output；调用者可以通过 opts 覆盖
    /// 错误信息、人类等待开始时间、subject 引用等无法从 run 推导的字段。
    async fn sync_step(
        &self,
        run: &WorkflowRun,
        compiled: &CompiledNode,
        opts: SyncStepOpts,
    ) -> WorkflowResult<()> {
        let view = build_step_view(run, compiled, opts);
        self.tracker.sync_step(run, &view).await
    }

    /// 等价于 `sync_step` 但不带任何 opts。
    async fn sync_step_basic(
        &self,
        run: &WorkflowRun,
        compiled: &CompiledNode,
    ) -> WorkflowResult<()> {
        self.sync_step(run, compiled, SyncStepOpts::default()).await
    }

    async fn sync_shard(
        &self,
        run: &WorkflowRun,
        for_each_id: &str,
        shard_index: u32,
        error: Option<String>,
    ) -> WorkflowResult<()> {
        let view = build_shard_view(run, for_each_id, shard_index, error);
        self.tracker.sync_map_shard(run, &view).await
    }

    async fn sync_thunk_dispatch(
        &self,
        run: &WorkflowRun,
        node_id: &str,
        execution_id: &str,
        thunk_obj_id: &str,
        attempt: u32,
        shard_index: Option<u32>,
    ) -> WorkflowResult<()> {
        let view = ThunkTaskView {
            node_id: node_id.to_string(),
            execution_id: execution_id.to_string(),
            thunk_obj_id: thunk_obj_id.to_string(),
            attempt,
            shard_index,
            requested_by: run.requested_by.clone(),
        };
        self.tracker.sync_thunk(run, &view).await
    }

    /// Agent 直接写入步骤输出，等价于 §3.4 `workflow.submit_step_output` 的语义：
    /// 若节点处于 WaitingHuman / Running / Failed 都接受，作为该节点的最终输出
    /// 推进状态机。在 TaskData 渠道中对应 `human_action.kind == "submit_output"`。
    pub async fn submit_step_output(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        node_id: &str,
        actor: &str,
        output: Value,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let compiled = workflow
            .nodes
            .get(node_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(node_id.to_string()))?;
        let state = run
            .node_states
            .get(node_id)
            .copied()
            .unwrap_or(NodeRunState::Pending);
        if !matches!(
            state,
            NodeRunState::Running
                | NodeRunState::WaitingHuman
                | NodeRunState::Failed
                | NodeRunState::Pending
        ) {
            return Err(WorkflowError::InvalidHumanAction {
                node_id: node_id.to_string(),
                action: "submit_output".to_string(),
            });
        }

        self.cancel_pending_executions(run, Some(node_id)).await;

        run.node_states
            .insert(node_id.to_string(), NodeRunState::Completed);
        run.node_outputs.insert(node_id.to_string(), output.clone());
        run.node_errors.remove(node_id);
        run.node_sources
            .insert(node_id.to_string(), "submitted".to_string());
        run.human_waiting_nodes.remove(node_id);
        self.activate_successors(workflow, run, node_id);

        let mut events = Vec::new();
        events.push(self.emit_event(
            run,
            "step.completed",
            Some(node_id.to_string()),
            actor,
            Some(json!({ "source": "submit_step_output" })),
        ));
        self.notify_par_branch_completed(workflow, run, node_id, &mut events)
            .await?;
        self.sync_step_basic(run, compiled).await?;
        self.refresh_run_status(workflow, run, &mut events);
        self.tracker.sync_run(run).await?;
        Ok(events)
    }

    /// Agent 上报进度，仅落事件 + 把 progress_message 写到 Step task 的 TaskData。
    /// 不修改节点状态。
    pub async fn report_step_progress(
        &self,
        _workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        node_id: &str,
        actor: &str,
        progress: Value,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let compiled = _workflow
            .nodes
            .get(node_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(node_id.to_string()))?;
        let progress_message = progress
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut events = Vec::new();
        events.push(self.emit_event(
            run,
            "step.progress",
            Some(node_id.to_string()),
            actor,
            Some(progress.clone()),
        ));
        self.sync_step(
            run,
            compiled,
            SyncStepOpts {
                progress_message,
                ..Default::default()
            },
        )
        .await?;
        Ok(events)
    }

    /// Agent 主动把当前 Step 切到 WaitingHuman，由 workflow 写 TaskData 等待用户操作。
    pub async fn request_human(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        node_id: &str,
        actor: &str,
        prompt: Option<String>,
        subject: Option<Value>,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let compiled = workflow
            .nodes
            .get(node_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(node_id.to_string()))?;

        self.cancel_pending_executions(run, Some(node_id)).await;

        run.node_states
            .insert(node_id.to_string(), NodeRunState::WaitingHuman);
        run.human_waiting_nodes.insert(node_id.to_string());
        let waiting_since = Utc::now().timestamp();

        let subject_value = subject.clone();
        let subject_obj_id = if let Some(value) = subject.as_ref() {
            Some(
                self.object_store
                    .put_json("workflow_subject", value)
                    .await?,
            )
        } else {
            None
        };

        let mut events = Vec::new();
        let payload = json!({
            "prompt": prompt,
            "subject": subject_value,
            "actor": actor,
        });
        events.push(self.emit_event(
            run,
            "step.waiting_human",
            Some(node_id.to_string()),
            actor,
            Some(payload),
        ));
        self.sync_step(
            run,
            compiled,
            SyncStepOpts {
                waiting_since: Some(waiting_since),
                subject_obj_id,
                subject_value,
                ..Default::default()
            },
        )
        .await?;
        self.refresh_run_status(workflow, run, &mut events);
        self.tracker.sync_run(run).await?;
        Ok(events)
    }

    /// 解释 §3.3 中 TaskData 上的 `human_action`，把它翻译成内部状态机动作。
    /// 用户在 TaskMgr UI 中点按钮 = 写一次 TaskData，service 监听 task_manager 的
    /// TaskData 变更后把整个 task_data JSON 喂给这个方法。
    pub async fn apply_task_data(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        task_data: &WorkflowStepTaskData,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let node_id = task_data.request.node_id.trim();
        if node_id.is_empty() {
            return Err(WorkflowError::Serialization(
                "task_data.request.node_id missing".to_string(),
            ));
        }
        if !task_data.request.run_id.is_empty() && task_data.request.run_id != run.run_id {
            return Err(WorkflowError::Serialization(format!(
                "task_data.request.run_id `{}` does not match run `{}`",
                task_data.request.run_id, run.run_id
            )));
        }

        let action = match task_data.human_action.as_ref() {
            Some(value) => value,
            None => return Ok(Vec::new()),
        };
        let TaskHumanAction {
            kind,
            payload,
            actor,
            ..
        } = action;
        let kind = kind.trim();
        if kind.is_empty() {
            return Err(WorkflowError::Serialization(
                "task_data.human_action.kind missing".to_string(),
            ));
        }
        let actor = actor.as_deref().unwrap_or("human").to_string();

        if kind == "submit_output" {
            let output = payload.clone().unwrap_or(Value::Null);
            return match self
                .submit_step_output(workflow, run, node_id, &actor, output)
                .await
            {
                Ok(events) => Ok(events),
                Err(err) => {
                    let _ = self
                        .tracker
                        .report_step_validation_error(run, node_id, &err.to_string())
                        .await;
                    Err(err)
                }
            };
        }

        let action_kind = match kind {
            "approve" => HumanActionKind::Approve,
            "modify" => HumanActionKind::Modify,
            "reject" => HumanActionKind::Reject,
            "retry" => HumanActionKind::Retry,
            "skip" => HumanActionKind::Skip,
            "abort" => HumanActionKind::Abort,
            "rollback" => HumanActionKind::Rollback,
            other => {
                let msg = format!("unknown human_action kind `{}`", other);
                let _ = self
                    .tracker
                    .report_step_validation_error(run, node_id, &msg)
                    .await;
                return Err(WorkflowError::InvalidHumanAction {
                    node_id: node_id.to_string(),
                    action: other.to_string(),
                });
            }
        };

        match self
            .handle_human_action(
                workflow,
                run,
                HumanAction {
                    node_id: node_id.to_string(),
                    action: action_kind,
                    payload: payload.clone(),
                    actor,
                },
            )
            .await
        {
            Ok(events) => Ok(events),
            Err(err) => {
                let _ = self
                    .tracker
                    .report_step_validation_error(run, node_id, &err.to_string())
                    .await;
                Err(err)
            }
        }
    }
}

#[derive(Default)]
struct SyncStepOpts {
    error: Option<String>,
    waiting_since: Option<i64>,
    subject_obj_id: Option<String>,
    subject_value: Option<Value>,
    progress_message: Option<String>,
}

fn build_step_view(run: &WorkflowRun, compiled: &CompiledNode, opts: SyncStepOpts) -> StepTaskView {
    let state = run
        .node_states
        .get(&compiled.id)
        .copied()
        .unwrap_or(NodeRunState::Pending);
    let attempt = run.node_attempts.get(&compiled.id).copied().unwrap_or(0);
    let executor = match &compiled.expr {
        Expr::Apply { executor, .. } => Some(executor.as_str().to_string()),
        _ => None,
    };
    let prompt = match &compiled.expr {
        Expr::Await { prompt, .. } => prompt.clone(),
        _ => None,
    };
    let output = if matches!(
        state,
        NodeRunState::Completed | NodeRunState::Skipped | NodeRunState::WaitingHuman
    ) {
        run.node_outputs.get(&compiled.id).cloned()
    } else {
        None
    };
    let error_detail = run.node_errors.get(&compiled.id).cloned();
    let error = opts.error.or_else(|| {
        error_detail
            .as_ref()
            .filter(|_| matches!(state, NodeRunState::Failed | NodeRunState::Aborted | NodeRunState::Cancelled))
            .and_then(|detail| detail.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    StepTaskView {
        node_id: compiled.id.clone(),
        name: compiled.name.clone(),
        state,
        attempt,
        executor,
        subject: opts.subject_value,
        subject_obj_id: opts.subject_obj_id,
        prompt,
        output_schema: compiled.output_schema.clone(),
        stakeholders: Vec::new(),
        progress_message: opts.progress_message,
        error,
        error_detail,
        output,
        source: run.node_sources.get(&compiled.id).cloned(),
        waiting_human_since: opts.waiting_since,
    }
}

fn bump_attempt(run: &mut WorkflowRun, node_id: &str) -> u32 {
    let attempt = run.node_attempts.entry(node_id.to_string()).or_insert(0);
    *attempt += 1;
    *attempt
}

fn step_timeout_secs(compiled: &CompiledNode) -> Option<u64> {
    match &compiled.expr {
        Expr::Apply { guards, .. } => guards.timeout.as_deref().and_then(parse_duration_secs),
        _ => None,
    }
}

/// `30s` / `5m` / `2h` / bare seconds.
pub fn parse_duration_secs(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (digits, unit) = raw.split_at(raw.trim_end_matches(|c: char| c.is_ascii_alphabetic()).len());
    let value: u64 = digits.trim().parse().ok()?;
    let multiplier = match unit.trim() {
        "" | "s" | "sec" | "secs" => 1,
        "m" | "min" | "mins" => 60,
        "h" | "hr" | "hours" => 3600,
        _ => return None,
    };
    Some(value * multiplier)
}

fn build_shard_view(
    run: &WorkflowRun,
    for_each_id: &str,
    shard_index: u32,
    error: Option<String>,
) -> MapShardTaskView {
    let map_state = run.map_states.get(for_each_id);
    let state = map_state
        .and_then(|s| s.shard_states.get(shard_index as usize).copied())
        .unwrap_or(NodeRunState::Pending);
    let attempt = map_state
        .and_then(|s| s.shard_attempts.get(shard_index as usize).copied())
        .unwrap_or(0);
    let item = map_state
        .and_then(|s| s.items.get(shard_index as usize).cloned())
        .unwrap_or(Value::Null);
    let output = map_state.and_then(|s| {
        let value = s.shard_outputs.get(shard_index as usize)?;
        if matches!(state, NodeRunState::Completed) {
            Some(value.clone())
        } else {
            None
        }
    });
    MapShardTaskView {
        for_each_id: for_each_id.to_string(),
        shard_index,
        state,
        attempt,
        item,
        output,
        error,
    }
}

fn extract_items(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::Array(items) => Some(items.clone()),
        Value::Object(map) => map.get("items").and_then(|inner| match inner {
            Value::Array(items) => Some(items.clone()),
            _ => None,
        }),
        _ => None,
    }
}

fn resolve_template_value(run: &WorkflowRun, value: &ValueTemplate) -> WorkflowResult<Value> {
    match value {
        ValueTemplate::Literal(value) => Ok(value.clone()),
        ValueTemplate::Reference(reference) => resolve_reference_value(run, reference),
        ValueTemplate::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| resolve_template_value(run, item))
                .collect::<WorkflowResult<Vec<_>>>()?,
        )),
        ValueTemplate::Object(map) => {
            let mut resolved = serde_json::Map::new();
            for (key, value) in map {
                resolved.insert(key.clone(), resolve_template_value(run, value)?);
            }
            Ok(Value::Object(resolved))
        }
    }
}

fn resolve_reference_value(
    run: &WorkflowRun,
    reference: &crate::types::RefPath,
) -> WorkflowResult<Value> {
    let mut current = run
        .node_outputs
        .get(&reference.node_id)
        .cloned()
        .ok_or_else(|| WorkflowError::ReferenceResolution(reference.as_string()))?;
    for segment in &reference.field_path {
        current = current
            .get(segment)
            .cloned()
            .ok_or_else(|| WorkflowError::ReferenceResolution(reference.as_string()))?;
    }
    Ok(current)
}

fn dependency_satisfied(
    workflow: &CompiledWorkflow,
    run: &WorkflowRun,
    node_id: &str,
    dependency: &str,
) -> bool {
    if matches!(
        run.node_states.get(dependency),
        Some(NodeRunState::Completed | NodeRunState::Skipped)
    ) {
        return true;
    }
    if let Some(parent) = workflow.nodes.get(dependency) {
        let is_running = matches!(run.node_states.get(dependency), Some(NodeRunState::Running));
        if !is_running {
            return false;
        }
        match &parent.expr {
            Expr::Par { branches, .. } if branches.iter().any(|b| b == node_id) => true,
            Expr::Map { steps, .. } if steps.first().map(String::as_str) == Some(node_id) => true,
            _ => false,
        }
    } else {
        false
    }
}

fn retry_policy(compiled: &CompiledNode) -> RetryPolicy {
    match &compiled.expr {
        Expr::Apply { guards, .. } => RetryPolicy::from(guards.retry.clone()),
        _ => RetryPolicy::default(),
    }
}

/// Cache key of an orchestrator-side adapter step: the executor string is
/// its only identity (no FunctionObject, no fingerprint).
fn adapter_cache_key(compiled: &CompiledNode, resolved_input: &Value) -> String {
    let Expr::Apply { executor, .. } = &compiled.expr else {
        return String::new();
    };
    let payload = json!({
        "executor": executor.as_str(),
        "input": resolved_input,
    });
    deterministic_object_id("adaptercache", &payload)
        .unwrap_or_default()
        .trim_start_matches("adaptercache:")
        .to_string()
}

/// 从 `Apply` 节点取出可用于构造 ThunkObject 的 `fun_id`。一期只有
/// `func::<objid>`（即 `ExecutorRef::Actual` 且属于 FunctionObject 命名空间）
/// 会走调度器路径；其它 namespace 还没有 adapter 的话报明确错误，避免静默
/// 落到调度器上。语义链接（`/agent/`、`/skill/`、`/tool/`）必须先经 registry
/// 展开，否则不应出现在投递路径上。
fn require_function_object<'a>(
    node_id: &str,
    executor: &ExecutorRef,
    fun_id: Option<&'a str>,
) -> WorkflowResult<&'a str> {
    if let Some(id) = fun_id {
        return Ok(id);
    }
    match executor {
        ExecutorRef::SemanticPath(value) => Err(WorkflowError::UnresolvedSemanticExecutor {
            node_id: node_id.to_string(),
            executor: value.clone(),
        }),
        ExecutorRef::Actual(value) => {
            let namespace = executor
                .namespace()
                .map(str::to_string)
                .unwrap_or_else(|| "<unknown>".to_string());
            Err(WorkflowError::ExecutorNamespaceNotImplemented {
                node_id: node_id.to_string(),
                executor: value.clone(),
                namespace,
            })
        }
    }
}

#[cfg(test)]
mod tests;
