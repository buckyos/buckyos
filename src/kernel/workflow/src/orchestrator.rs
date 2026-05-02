use crate::compiler::{CompiledNode, CompiledWorkflow};
use crate::dispatcher::ThunkDispatcher;
use crate::dsl::RetryFallback;
use crate::error::{WorkflowError, WorkflowResult};
use crate::executor_adapter::{ExecutorAdapter, ExecutorRegistry};
use crate::object_store::{deterministic_object_id, WorkflowObjectStore};
use crate::runtime::{
    EventEnvelope, HumanAction, HumanActionKind, HumanWait, MapState, NodeRunState, ParJoin,
    ParState, PendingThunk, RunStatus, WorkflowRun,
};
use crate::task_tracker::{
    MapShardTaskView, StepTaskView, ThunkTaskView, WorkflowTaskTracker,
};
use crate::types::{AwaitKind, Expr, ExecutorRef, JoinStrategy, RetryPolicy, ValueTemplate};
use buckyos_api::{ThunkExecutionResult, ThunkExecutionStatus, ThunkObject};
use chrono::Utc;
use ndn_lib::ObjId;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use uuid::Uuid;

pub struct WorkflowOrchestrator<D, O, T> {
    dispatcher: Arc<D>,
    object_store: Arc<O>,
    tracker: Arc<T>,
    executor_registry: Arc<ExecutorRegistry>,
}

impl<D, O, T> WorkflowOrchestrator<D, O, T>
where
    D: ThunkDispatcher + 'static,
    O: WorkflowObjectStore + 'static,
    T: WorkflowTaskTracker + 'static,
{
    pub fn new(dispatcher: Arc<D>, object_store: Arc<O>, tracker: Arc<T>) -> Self {
        Self {
            dispatcher,
            object_store,
            tracker,
            executor_registry: Arc::new(ExecutorRegistry::new()),
        }
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
        let now = Utc::now().timestamp();
        let mut node_states = BTreeMap::new();
        for node_id in workflow.nodes.keys() {
            node_states.insert(node_id.clone(), NodeRunState::Pending);
        }

        let mut run = WorkflowRun {
            run_id: format!("run-{}", Uuid::new_v4()),
            workflow_id: workflow.workflow_id.clone(),
            workflow_name: workflow.workflow_name.clone(),
            plan_version: 1,
            status: RunStatus::Created,
            node_states,
            node_outputs: BTreeMap::new(),
            node_attempts: BTreeMap::new(),
            branch_iterations: BTreeMap::new(),
            pending_thunks: BTreeMap::new(),
            activated_nodes: workflow.graph.start_nodes.iter().cloned().collect(),
            metrics: BTreeMap::new(),
            human_waiting_nodes: BTreeSet::new(),
            map_states: BTreeMap::new(),
            par_states: BTreeMap::new(),
            seq: 0,
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
        if run.status == RunStatus::Created {
            run.status = RunStatus::Running;
            events.push(self.emit_event(run, "run.started", None, "engine", None));
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
                        self.enter_par(workflow, run, compiled, &mut events)
                            .await?;
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

    pub async fn handle_thunk_result(
        &self,
        workflow: &CompiledWorkflow,
        run: &mut WorkflowRun,
        result: ThunkExecutionResult,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let mut events = Vec::new();
        let pending = run
            .pending_thunks
            .remove(result.thunk_obj_id.to_string().as_str())
            .ok_or_else(|| WorkflowError::MissingPendingThunk(result.thunk_obj_id.to_string()))?;
        let node_id = pending.node_id;
        let compiled = workflow
            .nodes
            .get(&node_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(node_id.clone()))?;

        match result.status {
            ThunkExecutionStatus::Waiting | ThunkExecutionStatus::Dispatched => {
                run.pending_thunks.insert(
                    result.thunk_obj_id.to_string(),
                    PendingThunk {
                        node_id: node_id.clone(),
                        thunk_obj_id: result.thunk_obj_id.to_string(),
                        attempt: pending.attempt,
                        shard_index: pending.shard_index,
                    },
                );
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
                        "task_id": result.task_id,
                        "status": result.status,
                    })),
                ));
            }
            ThunkExecutionStatus::Success => {
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
                    self.object_store
                        .put_json("workflow_result", &output)
                        .await?
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
                    run.metrics.insert(node_id.clone(), result.metrics.clone());
                    if is_idempotent(compiled) {
                        let cache_key =
                            cache_key(compiled, &self.resolve_apply_input(run, compiled)?);
                        self.object_store
                            .put_json_with_id(
                                &cache_key,
                                &json!({
                                    "result_obj_id": output_id,
                                    "result": output,
                                    "metrics": result.metrics,
                                }),
                            )
                            .await?;
                    }
                    self.activate_successors(workflow, run, &node_id);
                    self.notify_par_branch_completed(workflow, run, &node_id, &mut events)
                        .await?;
                    events.push(self.emit_event(
                        run,
                        "step.completed",
                        Some(node_id.clone()),
                        "engine",
                        Some(json!({ "result_obj_id": output_id })),
                    ));
                    self.sync_step_basic(run, compiled).await?;
                }
            }
            ThunkExecutionStatus::Failed | ThunkExecutionStatus::Cancelled => {
                if let Some(shard_index) = pending.shard_index {
                    self.handle_shard_failed(
                        workflow,
                        run,
                        &node_id,
                        shard_index,
                        result.error.clone(),
                        &mut events,
                    )
                    .await?;
                    self.refresh_run_status(workflow, run, &mut events);
                    self.tracker.sync_run(run).await?;
                    return Ok(events);
                }
                let policy = retry_policy(compiled);
                let attempt = run.node_attempts.get(&node_id).copied().unwrap_or(1);
                let mut waiting_since = None;
                if attempt < policy.max_attempts && result.status == ThunkExecutionStatus::Failed {
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::Retrying);
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::Pending);
                    events.push(self.emit_event(
                        run,
                        "step.retrying",
                        Some(node_id.clone()),
                        "engine",
                        Some(json!({ "attempt": attempt + 1, "error": result.error })),
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
                        Some(json!({ "reason": "retries_exhausted", "error": result.error })),
                    ));
                } else {
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::Failed);
                    events.push(self.emit_event(
                        run,
                        "step.failed",
                        Some(node_id.clone()),
                        "engine",
                        Some(json!({ "error": result.error })),
                    ));
                }
                self.sync_step(
                    run,
                    compiled,
                    SyncStepOpts {
                        error: result.error.clone(),
                        waiting_since,
                        ..Default::default()
                    },
                )
                .await?;
            }
        }

        self.refresh_run_status(workflow, run, &mut events);
        self.tracker.sync_run(run).await?;
        Ok(events)
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
        if compiled.idempotent {
            if let Some(cached) = self.lookup_cache(compiled, &resolved_input).await? {
                run.node_states
                    .insert(compiled.id.clone(), NodeRunState::Completed);
                run.node_outputs.insert(compiled.id.clone(), cached.clone());
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
                self.sync_step_basic(run, compiled).await?;
                return Ok(());
            }
        }

        let next_attempt = {
            let attempt = run.node_attempts.entry(compiled.id.clone()).or_insert(0);
            *attempt += 1;
            *attempt
        };

        // 编排器侧 Apply 直执行通道：命中 registry 的 executor（一期主要是
        // service:: / http:: / appservice::）跳过 Thunk 调度器，由编排器同步
        // 调用 adapter 并把结果直接回填到 node_outputs。
        if let Some(adapter) = self.adapter_for(compiled) {
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

        let (thunk_obj_id, thunk) = build_thunk(run, compiled, resolved_input, next_attempt)?;
        self.object_store
            .put_json(
                "workflow_thunk",
                &serde_json::to_value(&thunk)
                    .map_err(|err| WorkflowError::Serialization(err.to_string()))?,
            )
            .await?;
        self.dispatcher
            .schedule_thunk(&thunk_obj_id, &thunk)
            .await?;
        run.pending_thunks.insert(
            thunk_obj_id.clone(),
            PendingThunk {
                node_id: compiled.id.clone(),
                thunk_obj_id: thunk_obj_id.clone(),
                attempt: next_attempt,
                shard_index: None,
            },
        );
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Running);
        events.push(self.emit_event(
            run,
            "step.started",
            Some(compiled.id.clone()),
            "engine",
            None,
        ));
        self.sync_step_basic(run, compiled).await?;
        self.sync_thunk_dispatch(run, &compiled.id, &thunk_obj_id, next_attempt, None)
            .await?;
        Ok(())
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
        let output_id = self
            .object_store
            .put_json("workflow_result", &output)
            .await?;
        run.node_states
            .insert(compiled.id.clone(), NodeRunState::Completed);
        run.node_outputs
            .insert(compiled.id.clone(), output.clone());
        run.metrics
            .insert(compiled.id.clone(), metrics.clone());
        if is_idempotent(compiled) {
            let key = cache_key(compiled, resolved_input);
            self.object_store
                .put_json_with_id(
                    &key,
                    &json!({
                        "result_obj_id": output_id,
                        "result": output,
                        "metrics": metrics,
                    }),
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
        let policy = retry_policy(compiled);
        let error_message = err.to_string();
        let node_id = compiled.id.clone();
        let mut waiting_since = None;
        if attempt < policy.max_attempts {
            // 重新置为 Pending 让下一轮 tick 再次进入 schedule_apply（attempt 已自增过）。
            run.node_states
                .insert(node_id.clone(), NodeRunState::Pending);
            events.push(self.emit_event(
                run,
                "step.retrying",
                Some(node_id.clone()),
                "engine",
                Some(json!({ "attempt": attempt + 1, "error": error_message })),
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
                Some(json!({ "reason": "retries_exhausted", "error": error_message })),
            ));
        } else {
            run.node_states
                .insert(node_id.clone(), NodeRunState::Failed);
            events.push(self.emit_event(
                run,
                "step.failed",
                Some(node_id.clone()),
                "engine",
                Some(json!({ "error": error_message })),
            ));
        }
        self.sync_step(
            run,
            compiled,
            SyncStepOpts {
                error: Some(error_message),
                waiting_since,
                ..Default::default()
            },
        )
        .await?;
        Ok(())
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
            Some(value) if !value.is_null() => {
                Some(self.object_store.put_json("workflow_subject", value).await?)
            }
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

    async fn lookup_cache(
        &self,
        compiled: &CompiledNode,
        resolved_input: &Value,
    ) -> WorkflowResult<Option<Value>> {
        let cache_key = cache_key(compiled, resolved_input);
        let Some(value) = self.object_store.get_json(&cache_key).await? else {
            return Ok(None);
        };
        Ok(value.get("result").cloned())
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
                        let pending_for_branch: Vec<String> = run
                            .pending_thunks
                            .iter()
                            .filter(|(_, info)| {
                                info.node_id == *branch_id && info.shard_index.is_none()
                            })
                            .map(|(id, _)| id.clone())
                            .collect();
                        for thunk_id in pending_for_branch {
                            let _ = self.dispatcher.cancel_thunk(&thunk_id).await;
                            run.pending_thunks.remove(&thunk_id);
                        }
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
        let items = extract_items(&collection_value).ok_or_else(|| {
            WorkflowError::ForEachItemsType {
                node_id: compiled.id.clone(),
                actual: format!("{:?}", collection_value),
            }
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
            self.complete_map(workflow, run, &compiled.id, events).await?;
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
        let (max_concurrency, total, body_step_id) = {
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

            let (thunk_obj_id, thunk) = build_shard_thunk(
                run,
                body_node,
                for_each_id,
                &body_step_id,
                index as u32,
                resolved_input,
                attempt,
            )?;
            self.object_store
                .put_json(
                    "workflow_thunk",
                    &serde_json::to_value(&thunk)
                        .map_err(|err| WorkflowError::Serialization(err.to_string()))?,
                )
                .await?;
            self.dispatcher
                .schedule_thunk(&thunk_obj_id, &thunk)
                .await?;
            run.pending_thunks.insert(
                thunk_obj_id.clone(),
                PendingThunk {
                    node_id: for_each_id.to_string(),
                    thunk_obj_id,
                    attempt,
                    shard_index: Some(index as u32),
                },
            );
            if let Some(state) = run.map_states.get_mut(for_each_id) {
                state.shard_states[index] = NodeRunState::Running;
            }
            self.sync_shard(run, for_each_id, index as u32, None).await?;
            self.sync_thunk_dispatch(
                run,
                for_each_id,
                run.pending_thunks
                    .iter()
                    .find(|(_, info)| {
                        info.node_id == for_each_id && info.shard_index == Some(index as u32)
                    })
                    .map(|(id, _)| id.as_str())
                    .unwrap_or(""),
                attempt,
                Some(index as u32),
            )
            .await?;
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
                self.complete_map(workflow, run, for_each_id, events).await?;
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
            self.complete_map(workflow, run, for_each_id, events).await?;
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
        thunk_obj_id: &str,
        attempt: u32,
        shard_index: Option<u32>,
    ) -> WorkflowResult<()> {
        let view = ThunkTaskView {
            node_id: node_id.to_string(),
            thunk_obj_id: thunk_obj_id.to_string(),
            attempt,
            shard_index,
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

        // 取消任何还挂着的 thunk（agent 直接给结果就不再等调度器）。
        let pending_for_node: Vec<String> = run
            .pending_thunks
            .iter()
            .filter(|(_, info)| info.node_id == node_id && info.shard_index.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        for thunk_id in pending_for_node {
            let _ = self.dispatcher.cancel_thunk(&thunk_id).await;
            run.pending_thunks.remove(&thunk_id);
        }

        run.node_states
            .insert(node_id.to_string(), NodeRunState::Completed);
        run.node_outputs.insert(node_id.to_string(), output.clone());
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

        // 取消任何还挂着的 thunk。
        let pending_for_node: Vec<String> = run
            .pending_thunks
            .iter()
            .filter(|(_, info)| info.node_id == node_id && info.shard_index.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        for thunk_id in pending_for_node {
            let _ = self.dispatcher.cancel_thunk(&thunk_id).await;
            run.pending_thunks.remove(&thunk_id);
        }

        run.node_states
            .insert(node_id.to_string(), NodeRunState::WaitingHuman);
        run.human_waiting_nodes.insert(node_id.to_string());
        let waiting_since = Utc::now().timestamp();

        let subject_value = subject.clone();
        let subject_obj_id = if let Some(value) = subject.as_ref() {
            Some(self.object_store.put_json("workflow_subject", value).await?)
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
        task_data: &Value,
    ) -> WorkflowResult<Vec<EventEnvelope>> {
        let workflow_meta = task_data
            .get("workflow")
            .ok_or_else(|| WorkflowError::Serialization(
                "task_data missing `workflow` descriptor".to_string(),
            ))?;
        let node_id = workflow_meta
            .get("node_id")
            .and_then(Value::as_str)
            .ok_or_else(|| WorkflowError::Serialization(
                "task_data.workflow.node_id missing".to_string(),
            ))?
            .to_string();
        if let Some(declared_run) = workflow_meta.get("run_id").and_then(Value::as_str) {
            if declared_run != run.run_id {
                return Err(WorkflowError::Serialization(format!(
                    "task_data.workflow.run_id `{}` does not match run `{}`",
                    declared_run, run.run_id
                )));
            }
        }

        let action = match task_data.get("human_action") {
            Some(value) => value,
            None => return Ok(Vec::new()),
        };
        let kind = action
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| WorkflowError::Serialization(
                "task_data.human_action.kind missing".to_string(),
            ))?
            .to_string();
        let actor = action
            .get("actor")
            .and_then(Value::as_str)
            .unwrap_or("human")
            .to_string();
        let payload = action.get("payload").cloned();

        if kind == "submit_output" {
            let output = payload.clone().unwrap_or(Value::Null);
            return match self
                .submit_step_output(workflow, run, &node_id, &actor, output)
                .await
            {
                Ok(events) => Ok(events),
                Err(err) => {
                    let _ = self
                        .tracker
                        .report_step_validation_error(run, &node_id, &err.to_string())
                        .await;
                    Err(err)
                }
            };
        }

        let action_kind = match kind.as_str() {
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
                    .report_step_validation_error(run, &node_id, &msg)
                    .await;
                return Err(WorkflowError::InvalidHumanAction {
                    node_id,
                    action: other.to_string(),
                });
            }
        };

        match self
            .handle_human_action(
                workflow,
                run,
                HumanAction {
                    node_id: node_id.clone(),
                    action: action_kind,
                    payload,
                    actor,
                },
            )
            .await
        {
            Ok(events) => Ok(events),
            Err(err) => {
                let _ = self
                    .tracker
                    .report_step_validation_error(run, &node_id, &err.to_string())
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

fn build_step_view(
    run: &WorkflowRun,
    compiled: &CompiledNode,
    opts: SyncStepOpts,
) -> StepTaskView {
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
        error: opts.error,
        output,
        waiting_human_since: opts.waiting_since,
    }
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

fn build_thunk(
    run: &WorkflowRun,
    compiled: &CompiledNode,
    resolved_input: Value,
    attempt: u32,
) -> WorkflowResult<(String, ThunkObject)> {
    let Expr::Apply {
        executor,
        fun_id,
        idempotent: _,
        guards: _,
        ..
    } = &compiled.expr
    else {
        return Err(WorkflowError::UnsupportedNode(compiled.id.clone()));
    };
    let fun_id = require_function_object(&compiled.id, executor, fun_id.as_deref())?;
    let params = match resolved_input {
        Value::Object(map) => map.into_iter().collect(),
        other => {
            return Err(WorkflowError::Serialization(format!(
                "resolved thunk params for node `{}` must be a JSON object, got {}",
                compiled.id, other
            )));
        }
    };
    let metadata = json!({
        "run_id": run.run_id.clone(),
        "node_id": compiled.id.clone(),
        "attempt": attempt,
    });
    let thunk_without_id = json!({
        "fun_id": fun_id,
        "params": params,
        "metadata": metadata,
    });
    let thunk_obj_id = deterministic_object_id("thunk", &thunk_without_id)?;

    Ok((
        thunk_obj_id,
        ThunkObject {
            fun_id: ObjId::new(format!("func:{}", fun_id).as_str()).map_err(|err| {
                WorkflowError::Serialization(format!("invalid fun_id `{}`: {}", fun_id, err))
            })?,
            params,
            metadata,
        },
    ))
}

fn build_shard_thunk(
    run: &WorkflowRun,
    body_node: &CompiledNode,
    for_each_id: &str,
    body_step_id: &str,
    shard_index: u32,
    resolved_input: Value,
    attempt: u32,
) -> WorkflowResult<(String, ThunkObject)> {
    let Expr::Apply {
        executor, fun_id, ..
    } = &body_node.expr
    else {
        return Err(WorkflowError::ForEachBodyNotApply(
            for_each_id.to_string(),
            body_step_id.to_string(),
        ));
    };
    let fun_id = require_function_object(&body_node.id, executor, fun_id.as_deref())?;
    let params = match resolved_input {
        Value::Object(map) => map.into_iter().collect::<HashMap<_, _>>(),
        other => {
            return Err(WorkflowError::Serialization(format!(
                "resolved shard params for `{}[{}]` must be a JSON object, got {}",
                for_each_id, shard_index, other
            )));
        }
    };
    let shard_node_id = format!("{}[{}]", for_each_id, shard_index);
    let metadata = json!({
        "run_id": run.run_id.clone(),
        "node_id": shard_node_id,
        "for_each_id": for_each_id,
        "body_step_id": body_step_id,
        "shard": { "index": shard_index },
        "attempt": attempt,
    });
    let thunk_without_id = json!({
        "fun_id": fun_id,
        "params": params,
        "metadata": metadata,
    });
    let thunk_obj_id = deterministic_object_id("thunk", &thunk_without_id)?;
    Ok((
        thunk_obj_id,
        ThunkObject {
            fun_id: ObjId::new(format!("func:{}", fun_id).as_str()).map_err(|err| {
                WorkflowError::Serialization(format!("invalid fun_id `{}`: {}", fun_id, err))
            })?,
            params,
            metadata,
        },
    ))
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

fn is_idempotent(compiled: &CompiledNode) -> bool {
    matches!(
        &compiled.expr,
        Expr::Apply {
            idempotent: true,
            ..
        }
    ) || compiled.idempotent
}

fn cache_key(compiled: &CompiledNode, resolved_input: &Value) -> String {
    let Expr::Apply {
        executor, fun_id, ..
    } = &compiled.expr
    else {
        return String::new();
    };
    // 缓存 key 必须能跨 Run 命中同一段函数。优先使用 FunctionObject 的 fun_id；
    // 对于 service:: / http:: / appservice:: / operator:: 这类编排器侧 adapter
    // 直接执行的 executor，没有 fun_id，用 executor 字符串本身做内容标识。
    let identity = match fun_id.as_deref() {
        Some(id) => id.to_string(),
        None => format!("ref:{}", executor.as_str()),
    };
    let payload = json!({
        "executor": executor.as_str(),
        "identity": identity,
        "input": resolved_input,
    });
    deterministic_object_id("workflow_cache", &payload).unwrap_or_default()
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
mod tests {
    use super::*;
    use crate::compiler::compile_workflow;
    use crate::dispatcher::InMemoryThunkDispatcher;
    use crate::dsl::{
        ControlNodeDefinition, EdgeDefinition, OutputMode, StepDefinition, StepType,
        WorkflowDefinition,
    };
    use crate::object_store::InMemoryObjectStore;
    use crate::task_tracker::NoopTaskTracker;
    use buckyos_api::{ThunkExecutionResult, ThunkExecutionStatus};
    use ndn_lib::ObjId;
    use serde_json::json;

    fn sample_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            schema_version: "0.2.0".to_string(),
            id: "wf-demo".to_string(),
            name: "Demo".to_string(),
            description: None,
            trigger: json!({"type":"manual"}),
            steps: vec![
                StepDefinition {
                    id: "plan".to_string(),
                    name: "Plan".to_string(),
                    executor: Some("func::agent.mia".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({
                        "type": "object",
                        "properties": {
                            "decision": { "type": "string", "enum": ["approved", "rejected"] }
                        },
                        "required": ["decision"]
                    }),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "review".to_string(),
                    name: "Review".to_string(),
                    executor: None,
                    step_type: StepType::HumanConfirm,
                    input: None,
                    input_schema: None,
                    output_schema: json!({
                        "type": "object",
                        "properties": {
                            "decision": { "type": "string", "enum": ["approved", "rejected"] },
                            "final_subject": {
                                "type": "object",
                                "properties": {
                                    "decision": { "type": "string", "enum": ["approved", "rejected"] }
                                },
                                "required": ["decision"]
                            }
                        },
                        "required": ["decision", "final_subject"]
                    }),
                    subject_ref: Some("${plan.output}".to_string()),
                    prompt: Some("review".to_string()),
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "approved_step".to_string(),
                    name: "Approved".to_string(),
                    executor: Some("func::skill.finalize".to_string()),
                    step_type: StepType::Autonomous,
                    input: Some(json!({
                        "source": "${review.output.final_subject}"
                    })),
                    input_schema: Some(json!({
                        "type": "object",
                        "properties": {
                            "source": {
                                "type": ["object", "null"]
                            }
                        }
                    })),
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: true,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
            ],
            nodes: vec![ControlNodeDefinition::Branch(
                crate::dsl::BranchNodeDefinition {
                    id: "decision_branch".to_string(),
                    on: "${review.output.decision}".to_string(),
                    paths: [
                        ("approved".to_string(), "approved_step".to_string()),
                        ("rejected".to_string(), "approved_step".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    max_iterations: 3,
                },
            )],
            edges: vec![
                EdgeDefinition {
                    from: "plan".to_string(),
                    to: Some("review".to_string()),
                },
                EdgeDefinition {
                    from: "review".to_string(),
                    to: Some("decision_branch".to_string()),
                },
                EdgeDefinition {
                    from: "approved_step".to_string(),
                    to: None,
                },
            ],
            guards: None,
            defs: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn orchestrator_runs_apply_await_match() {
        let compiled = compile_workflow(sample_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(NoopTaskTracker);
        let orchestrator = WorkflowOrchestrator::new(dispatcher.clone(), object_store, tracker);

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        let events = orchestrator.tick(&compiled, &mut run).await.unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "step.started"));

        let scheduled = dispatcher.scheduled().await;
        assert_eq!(scheduled.len(), 1);
        let thunk_id = scheduled[0].thunk_obj_id.clone();
        orchestrator
            .handle_thunk_result(
                &compiled,
                &mut run,
                ThunkExecutionResult {
                    thunk_obj_id: ObjId::new(thunk_id.as_str()).unwrap(),
                    task_id: "task-1".to_string(),
                    status: ThunkExecutionStatus::Success,
                    result_obj_id: None,
                    result: Some(json!({"decision":"approved"})),
                    error: None,
                    result_url: None,
                    metrics: Value::Null,
                },
            )
            .await
            .unwrap();
        let events = orchestrator.tick(&compiled, &mut run).await.unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "step.waiting_human"));

        orchestrator
            .handle_human_action(
                &compiled,
                &mut run,
                HumanAction {
                    node_id: "review".to_string(),
                    action: HumanActionKind::Approve,
                    payload: None,
                    actor: "human/test".to_string(),
                },
            )
            .await
            .unwrap();
        let events = orchestrator.tick(&compiled, &mut run).await.unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "step.started"));
    }

    fn parallel_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            schema_version: "0.2.0".to_string(),
            id: "wf-par".to_string(),
            name: "Par".to_string(),
            description: None,
            trigger: json!({"type":"manual"}),
            steps: vec![
                StepDefinition {
                    id: "seed".to_string(),
                    name: "Seed".to_string(),
                    executor: Some("func::skill.seed".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "branch_a".to_string(),
                    name: "A".to_string(),
                    executor: Some("func::skill.a".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "branch_b".to_string(),
                    name: "B".to_string(),
                    executor: Some("func::skill.b".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "join".to_string(),
                    name: "Join".to_string(),
                    executor: Some("func::skill.join".to_string()),
                    step_type: StepType::Autonomous,
                    input: Some(json!({
                        "all": "${par.output}"
                    })),
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
            ],
            nodes: vec![ControlNodeDefinition::Parallel(
                crate::dsl::ParallelNodeDefinition {
                    id: "par".to_string(),
                    branches: vec!["branch_a".to_string(), "branch_b".to_string()],
                    join: crate::dsl::JoinMode::All,
                    n: None,
                },
            )],
            edges: vec![
                EdgeDefinition {
                    from: "seed".to_string(),
                    to: Some("par".to_string()),
                },
                EdgeDefinition {
                    from: "par".to_string(),
                    to: Some("join".to_string()),
                },
                EdgeDefinition {
                    from: "join".to_string(),
                    to: None,
                },
            ],
            guards: None,
            defs: BTreeMap::new(),
        }
    }

    async fn finish_thunk<T>(
        orchestrator: &WorkflowOrchestrator<
            InMemoryThunkDispatcher,
            InMemoryObjectStore,
            T,
        >,
        compiled: &CompiledWorkflow,
        run: &mut WorkflowRun,
        thunk_id: &str,
        result: serde_json::Value,
    ) where
        T: crate::task_tracker::WorkflowTaskTracker + 'static,
    {
        orchestrator
            .handle_thunk_result(
                compiled,
                run,
                ThunkExecutionResult {
                    thunk_obj_id: ObjId::new(thunk_id).unwrap(),
                    task_id: "task".to_string(),
                    status: ThunkExecutionStatus::Success,
                    result_obj_id: None,
                    result: Some(result),
                    error: None,
                    result_url: None,
                    metrics: Value::Null,
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn orchestrator_runs_parallel_join_all() {
        let compiled = compile_workflow(parallel_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(NoopTaskTracker);
        let orchestrator =
            WorkflowOrchestrator::new(dispatcher.clone(), object_store, tracker);

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();

        let scheduled = dispatcher.scheduled().await;
        assert_eq!(scheduled.len(), 1);
        let seed_thunk = scheduled[0].thunk_obj_id.clone();
        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &seed_thunk,
            json!({"ok": true}),
        )
        .await;

        orchestrator.tick(&compiled, &mut run).await.unwrap();
        let scheduled = dispatcher.scheduled().await;
        assert_eq!(scheduled.len(), 3, "branch_a + branch_b should be dispatched");

        let branch_thunks = scheduled[1..3].to_vec();
        for sched in &branch_thunks {
            finish_thunk(
                &orchestrator,
                &compiled,
                &mut run,
                &sched.thunk_obj_id,
                json!({"value": 1}),
            )
            .await;
        }

        orchestrator.tick(&compiled, &mut run).await.unwrap();
        assert_eq!(
            run.node_states.get("par"),
            Some(&NodeRunState::Completed),
            "par should be completed after all branches finished"
        );
        let par_output = run.node_outputs.get("par").cloned().unwrap_or(Value::Null);
        assert!(par_output.get("branch_a").is_some());
        assert!(par_output.get("branch_b").is_some());

        let scheduled_after = dispatcher.scheduled().await;
        assert_eq!(scheduled_after.len(), 4, "join step should be scheduled");
        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &scheduled_after[3].thunk_obj_id,
            json!({"done": true}),
        )
        .await;
        orchestrator.tick(&compiled, &mut run).await.unwrap();
        assert_eq!(run.status, RunStatus::Completed);
    }

    fn for_each_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            schema_version: "0.2.0".to_string(),
            id: "wf-map".to_string(),
            name: "Map".to_string(),
            description: None,
            trigger: json!({"type":"manual"}),
            steps: vec![
                StepDefinition {
                    id: "scan".to_string(),
                    name: "Scan".to_string(),
                    executor: Some("func::skill.fs".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({
                        "type": "object",
                        "properties": {
                            "element_schema": {"type":"object"},
                            "total_count": {"type":"integer"},
                            "items": {"type":"array"}
                        }
                    }),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::FiniteSeekable,
                    guards: None,
                },
                StepDefinition {
                    id: "ingest".to_string(),
                    name: "Ingest".to_string(),
                    executor: Some("func::skill.ingest".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "summary".to_string(),
                    name: "Summary".to_string(),
                    executor: Some("func::skill.summary".to_string()),
                    step_type: StepType::Autonomous,
                    input: Some(json!({
                        "results": "${loop.output}"
                    })),
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
            ],
            nodes: vec![ControlNodeDefinition::ForEach(
                crate::dsl::ForEachNodeDefinition {
                    id: "loop".to_string(),
                    items: "${scan.output}".to_string(),
                    steps: vec!["ingest".to_string()],
                    max_items: 10,
                    concurrency: 2,
                },
            )],
            edges: vec![
                EdgeDefinition {
                    from: "scan".to_string(),
                    to: Some("loop".to_string()),
                },
                EdgeDefinition {
                    from: "loop".to_string(),
                    to: Some("summary".to_string()),
                },
                EdgeDefinition {
                    from: "summary".to_string(),
                    to: None,
                },
            ],
            guards: None,
            defs: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn orchestrator_runs_for_each_with_concurrency() {
        let compiled = compile_workflow(for_each_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(NoopTaskTracker);
        let orchestrator =
            WorkflowOrchestrator::new(dispatcher.clone(), object_store, tracker);

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();

        let scheduled = dispatcher.scheduled().await;
        assert_eq!(scheduled.len(), 1, "scan should be scheduled first");
        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &scheduled[0].thunk_obj_id,
            json!({"items": [1, 2, 3]}),
        )
        .await;

        orchestrator.tick(&compiled, &mut run).await.unwrap();
        let scheduled = dispatcher.scheduled().await;
        assert_eq!(
            scheduled.len(),
            1 + 2,
            "for_each should schedule first 2 shards (concurrency=2)"
        );

        let shard_a = scheduled[1].thunk_obj_id.clone();
        let shard_b = scheduled[2].thunk_obj_id.clone();
        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &shard_a,
            json!({"index": 0}),
        )
        .await;

        let scheduled = dispatcher.scheduled().await;
        assert_eq!(
            scheduled.len(),
            1 + 3,
            "third shard should be dispatched after the first one completes"
        );

        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &shard_b,
            json!({"index": 1}),
        )
        .await;
        let shard_c = scheduled[3].thunk_obj_id.clone();
        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &shard_c,
            json!({"index": 2}),
        )
        .await;

        orchestrator.tick(&compiled, &mut run).await.unwrap();
        let loop_output = run.node_outputs.get("loop").cloned().unwrap_or(Value::Null);
        let loop_array = loop_output.as_array().expect("loop output should be array");
        assert_eq!(loop_array.len(), 3);
        assert_eq!(loop_array[0]["index"], 0);
        assert_eq!(loop_array[2]["index"], 2);

        let scheduled = dispatcher.scheduled().await;
        assert_eq!(scheduled.len(), 1 + 3 + 1, "summary should be scheduled");
        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &scheduled[4].thunk_obj_id,
            json!({"done": true}),
        )
        .await;
        orchestrator.tick(&compiled, &mut run).await.unwrap();
        assert_eq!(run.status, RunStatus::Completed);
    }

    fn service_only_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            schema_version: "0.2.0".to_string(),
            id: "wf-service-only".to_string(),
            name: "ServiceOnly".to_string(),
            description: None,
            trigger: json!({"type":"manual"}),
            steps: vec![
                StepDefinition {
                    id: "fetch".to_string(),
                    name: "Fetch".to_string(),
                    executor: Some("service::aicc.complete".to_string()),
                    step_type: StepType::Autonomous,
                    input: Some(json!({"prompt": "hello"})),
                    input_schema: None,
                    output_schema: json!({
                        "type":"object",
                        "properties": {
                            "answer": {"type":"string"}
                        },
                        "required": ["answer"]
                    }),
                    subject_ref: None,
                    prompt: None,
                    idempotent: false,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "notify".to_string(),
                    name: "Notify".to_string(),
                    executor: Some("http::msg-center.notify".to_string()),
                    step_type: StepType::Autonomous,
                    input: Some(json!({
                        "answer": "${fetch.output.answer}"
                    })),
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: false,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
            ],
            nodes: vec![],
            edges: vec![
                EdgeDefinition {
                    from: "fetch".to_string(),
                    to: Some("notify".to_string()),
                },
                EdgeDefinition {
                    from: "notify".to_string(),
                    to: None,
                },
            ],
            guards: None,
            defs: BTreeMap::new(),
        }
    }

    fn echo_registry() -> Arc<crate::executor_adapter::ExecutorRegistry> {
        let adapter = crate::executor_adapter::NamespaceAdapter::new(
            ["service", "http", "appservice"],
            |executor, input| {
                let executor = executor.clone();
                let input = input.clone();
                Box::pin(async move {
                    if executor.as_str() == "service::aicc.complete" {
                        Ok(json!({ "answer": "ok", "echo": input }))
                    } else {
                        Ok(json!({ "delivered": true, "echo": input }))
                    }
                })
            },
        );
        Arc::new(crate::executor_adapter::ExecutorRegistry::new().with(Arc::new(adapter)))
    }

    #[tokio::test]
    async fn orchestrator_runs_service_and_http_directly() {
        let compiled = compile_workflow(service_only_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(NoopTaskTracker);
        let orchestrator = WorkflowOrchestrator::new(dispatcher.clone(), object_store, tracker)
            .with_executor_registry(echo_registry());

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        let _events = orchestrator.tick(&compiled, &mut run).await.unwrap();

        // 一次 tick 即可全程跑完，不应有任何 Thunk 投递到调度器。
        let scheduled = dispatcher.scheduled().await;
        assert!(
            scheduled.is_empty(),
            "service::/http:: should not go through dispatcher, got {:?}",
            scheduled
        );
        assert!(run.pending_thunks.is_empty());
        assert_eq!(run.status, RunStatus::Completed);
        assert_eq!(
            run.node_states.get("fetch"),
            Some(&NodeRunState::Completed)
        );
        assert_eq!(
            run.node_states.get("notify"),
            Some(&NodeRunState::Completed)
        );
        let fetch_out = run.node_outputs.get("fetch").cloned().unwrap_or(Value::Null);
        assert_eq!(fetch_out["answer"], "ok");
        let notify_out = run.node_outputs.get("notify").cloned().unwrap_or(Value::Null);
        assert_eq!(notify_out["delivered"], true);
        assert_eq!(notify_out["echo"]["answer"], "ok");
    }

    #[tokio::test]
    async fn orchestrator_direct_apply_falls_back_when_no_adapter() {
        // 没有 registry 时仍应走 Thunk 路径——保持原行为。
        let compiled = compile_workflow(service_only_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(NoopTaskTracker);
        let orchestrator = WorkflowOrchestrator::new(dispatcher.clone(), object_store, tracker);

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        let res = orchestrator.tick(&compiled, &mut run).await;
        // service:: 没有 fun_id 也没有 adapter，build_thunk 应明确报 namespace 未实现，
        // 而不是静默落到调度器。
        assert!(matches!(
            res,
            Err(WorkflowError::ExecutorNamespaceNotImplemented { .. })
        ));
    }

    fn http_for_each_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            schema_version: "0.2.0".to_string(),
            id: "wf-map-http".to_string(),
            name: "MapHttp".to_string(),
            description: None,
            trigger: json!({"type":"manual"}),
            steps: vec![
                StepDefinition {
                    id: "scan".to_string(),
                    name: "Scan".to_string(),
                    executor: Some("service::fs_index.scan".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({
                        "type":"object",
                        "properties": {
                            "element_schema": {"type":"object"},
                            "total_count": {"type":"integer"},
                            "items": {"type":"array"}
                        }
                    }),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::FiniteSeekable,
                    guards: None,
                },
                StepDefinition {
                    id: "classify".to_string(),
                    name: "Classify".to_string(),
                    executor: Some("http::file-classifier.classify".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "report".to_string(),
                    name: "Report".to_string(),
                    executor: Some("service::msg_center.notify_user".to_string()),
                    step_type: StepType::Autonomous,
                    input: Some(json!({"shards": "${loop.output}"})),
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
            ],
            nodes: vec![ControlNodeDefinition::ForEach(
                crate::dsl::ForEachNodeDefinition {
                    id: "loop".to_string(),
                    items: "${scan.output}".to_string(),
                    steps: vec!["classify".to_string()],
                    max_items: 10,
                    concurrency: 2,
                },
            )],
            edges: vec![
                EdgeDefinition {
                    from: "scan".to_string(),
                    to: Some("loop".to_string()),
                },
                EdgeDefinition {
                    from: "loop".to_string(),
                    to: Some("report".to_string()),
                },
                EdgeDefinition {
                    from: "report".to_string(),
                    to: None,
                },
            ],
            guards: None,
            defs: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn orchestrator_direct_apply_runs_for_each_via_adapter() {
        let compiled = compile_workflow(http_for_each_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(NoopTaskTracker);

        let adapter = crate::executor_adapter::NamespaceAdapter::new(
            ["service", "http"],
            |executor, input| {
                let executor = executor.clone();
                let input = input.clone();
                Box::pin(async move {
                    match executor.as_str() {
                        "service::fs_index.scan" => Ok(json!({"items": [
                            {"path": "/a"}, {"path": "/b"}, {"path": "/c"}
                        ]})),
                        "http::file-classifier.classify" => {
                            let item = input.get("_item").cloned().unwrap_or(Value::Null);
                            let index = input.get("_index").cloned().unwrap_or(Value::Null);
                            Ok(json!({"index": index, "kind": "doc", "item": item}))
                        }
                        "service::msg_center.notify_user" => Ok(json!({"sent": true})),
                        other => Err(WorkflowError::Dispatcher(format!(
                            "unexpected executor `{}`",
                            other
                        ))),
                    }
                })
            },
        );
        let registry = Arc::new(
            crate::executor_adapter::ExecutorRegistry::new().with(Arc::new(adapter)),
        );
        let orchestrator = WorkflowOrchestrator::new(dispatcher.clone(), object_store, tracker)
            .with_executor_registry(registry);

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();

        let scheduled = dispatcher.scheduled().await;
        assert!(
            scheduled.is_empty(),
            "no thunk should be dispatched, got {:?}",
            scheduled
        );
        assert!(run.pending_thunks.is_empty());
        assert_eq!(run.status, RunStatus::Completed);

        let loop_output = run.node_outputs.get("loop").cloned().unwrap_or(Value::Null);
        let arr = loop_output.as_array().expect("loop output should be array");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["index"], 0);
        assert_eq!(arr[2]["index"], 2);
        assert_eq!(arr[1]["kind"], "doc");

        assert_eq!(
            run.node_states.get("report"),
            Some(&NodeRunState::Completed)
        );
    }

    #[tokio::test]
    async fn orchestrator_direct_apply_failure_falls_back_to_human() {
        let compiled = compile_workflow(service_only_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(NoopTaskTracker);

        let failing = crate::executor_adapter::NamespaceAdapter::new(
            ["service", "http"],
            |_executor, _input| {
                Box::pin(async move {
                    Err(WorkflowError::Dispatcher("boom".to_string()))
                })
            },
        );
        let registry =
            Arc::new(crate::executor_adapter::ExecutorRegistry::new().with(Arc::new(failing)));
        let orchestrator = WorkflowOrchestrator::new(dispatcher.clone(), object_store, tracker)
            .with_executor_registry(registry);

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();

        // 第一步默认 retry 1 次 + Human fallback，所以应该停在 WaitingHuman。
        assert_eq!(run.status, RunStatus::WaitingHuman);
        assert_eq!(
            run.node_states.get("fetch"),
            Some(&NodeRunState::WaitingHuman)
        );
        assert!(dispatcher.scheduled().await.is_empty());
    }

    // -------- §6.3 / §3.3 任务树 + TaskData 路径 --------

    #[tokio::test]
    async fn tracker_records_step_thunk_and_human_wait_views() {
        let compiled = compile_workflow(sample_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(crate::task_tracker::RecordingTaskTracker::new());
        let orchestrator = WorkflowOrchestrator::new(
            dispatcher.clone(),
            object_store.clone(),
            tracker.clone(),
        );

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();

        // 第一个 Apply 节点应当至少产生过一次 Running 视图，并被 dispatcher 投递了一个 thunk
        let plan_view = tracker
            .step(&run.run_id, "plan")
            .await
            .expect("plan step view should exist");
        assert_eq!(plan_view.attempt, 1);
        assert_eq!(plan_view.executor.as_deref(), Some("func::agent.mia"));

        let scheduled = dispatcher.scheduled().await;
        assert_eq!(scheduled.len(), 1);
        let plan_thunk_id = scheduled[0].thunk_obj_id.clone();
        let thunk_view = tracker
            .thunk(&run.run_id, &plan_thunk_id)
            .await
            .expect("thunk task view should exist for dispatched thunk");
        assert_eq!(thunk_view.node_id, "plan");
        assert!(thunk_view.shard_index.is_none());

        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &plan_thunk_id,
            json!({"decision":"approved"}),
        )
        .await;
        orchestrator.tick(&compiled, &mut run).await.unwrap();

        // review 是 human_confirm 节点，应进入 WaitingHuman 并带上 subject_obj_id / waiting_since
        let review_view = tracker
            .step(&run.run_id, "review")
            .await
            .expect("review step view should exist after entering human wait");
        assert!(matches!(review_view.state, NodeRunState::WaitingHuman));
        assert!(review_view.waiting_human_since.is_some());
        assert!(review_view.subject_obj_id.is_some());
        // 其值应当确实存在于 object_store，而不是凭空构造
        let stored = object_store
            .get_json(review_view.subject_obj_id.as_ref().unwrap())
            .await
            .unwrap();
        assert!(stored.is_some());
    }

    #[tokio::test]
    async fn apply_task_data_translates_human_action_into_state_machine() {
        let compiled = compile_workflow(sample_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(crate::task_tracker::RecordingTaskTracker::new());
        let orchestrator = WorkflowOrchestrator::new(
            dispatcher.clone(),
            object_store,
            tracker.clone(),
        );

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();
        let plan_thunk = dispatcher.scheduled().await[0].thunk_obj_id.clone();
        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &plan_thunk,
            json!({"decision":"approved"}),
        )
        .await;
        orchestrator.tick(&compiled, &mut run).await.unwrap();
        assert_eq!(
            run.node_states.get("review"),
            Some(&NodeRunState::WaitingHuman)
        );

        // 模拟 TaskMgr UI 的 TaskData 写入：用户点了 "approve"
        let task_data = json!({
            "workflow": {
                "run_id": run.run_id,
                "node_id": "review",
            },
            "human_action": {
                "kind": "approve",
                "actor": "user-A",
            }
        });
        let events = orchestrator
            .apply_task_data(&compiled, &mut run, &task_data)
            .await
            .unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "human.action"));
        assert_eq!(
            run.node_states.get("review"),
            Some(&NodeRunState::Completed)
        );
    }

    #[tokio::test]
    async fn apply_task_data_validation_failure_records_last_error() {
        let compiled = compile_workflow(sample_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(crate::task_tracker::RecordingTaskTracker::new());
        let orchestrator =
            WorkflowOrchestrator::new(dispatcher, object_store, tracker.clone());

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();

        // plan 节点根本还没进入 WaitingHuman，approve 应当报错并写回 last_error
        let task_data = json!({
            "workflow": {
                "run_id": run.run_id,
                "node_id": "plan",
            },
            "human_action": {
                "kind": "approve",
                "actor": "user-A",
            }
        });
        let result = orchestrator
            .apply_task_data(&compiled, &mut run, &task_data)
            .await;
        assert!(result.is_err());
        let errors = tracker.validation_errors("plan").await;
        assert!(!errors.is_empty(), "tracker should record validation error");
    }

    #[tokio::test]
    async fn apply_task_data_rejects_run_id_mismatch() {
        let compiled = compile_workflow(sample_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(crate::task_tracker::RecordingTaskTracker::new());
        let orchestrator =
            WorkflowOrchestrator::new(dispatcher, object_store, tracker.clone());
        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();

        let task_data = json!({
            "workflow": {
                "run_id": "run-other",
                "node_id": "plan",
            },
            "human_action": {
                "kind": "approve",
            }
        });
        let result = orchestrator
            .apply_task_data(&compiled, &mut run, &task_data)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn submit_step_output_completes_running_node_via_agent() {
        let compiled = compile_workflow(sample_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(crate::task_tracker::RecordingTaskTracker::new());
        let orchestrator = WorkflowOrchestrator::new(
            dispatcher.clone(),
            object_store,
            tracker.clone(),
        );

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();
        assert_eq!(run.node_states.get("plan"), Some(&NodeRunState::Running));
        assert_eq!(dispatcher.scheduled().await.len(), 1);

        orchestrator
            .submit_step_output(
                &compiled,
                &mut run,
                "plan",
                "agent/mia",
                json!({"decision": "approved"}),
            )
            .await
            .unwrap();

        assert_eq!(
            run.node_states.get("plan"),
            Some(&NodeRunState::Completed)
        );
        // 应当取消挂起的 thunk
        assert!(run.pending_thunks.is_empty());
        assert_eq!(
            run.node_outputs.get("plan").and_then(|v| v.get("decision")),
            Some(&Value::String("approved".to_string()))
        );
    }

    #[tokio::test]
    async fn request_human_writes_subject_to_object_store() {
        let compiled = compile_workflow(sample_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(crate::task_tracker::RecordingTaskTracker::new());
        let orchestrator = WorkflowOrchestrator::new(
            dispatcher,
            object_store.clone(),
            tracker.clone(),
        );

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();

        orchestrator
            .request_human(
                &compiled,
                &mut run,
                "plan",
                "agent/mia",
                Some("please review".to_string()),
                Some(json!({"key": "value"})),
            )
            .await
            .unwrap();

        let view = tracker
            .step(&run.run_id, "plan")
            .await
            .expect("step view should exist");
        assert!(matches!(view.state, NodeRunState::WaitingHuman));
        assert!(view.waiting_human_since.is_some());
        let obj_id = view.subject_obj_id.expect("subject_obj_id should be set");
        let stored = object_store.get_json(&obj_id).await.unwrap();
        assert_eq!(stored, Some(json!({"key": "value"})));
        assert_eq!(run.status, RunStatus::WaitingHuman);
    }

    #[tokio::test]
    async fn for_each_records_map_shard_views() {
        let compiled = compile_workflow(for_each_workflow()).unwrap().workflow;
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(crate::task_tracker::RecordingTaskTracker::new());
        let orchestrator = WorkflowOrchestrator::new(
            dispatcher.clone(),
            object_store,
            tracker.clone(),
        );

        let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
        orchestrator.tick(&compiled, &mut run).await.unwrap();
        let scan_thunk = dispatcher.scheduled().await[0].thunk_obj_id.clone();
        finish_thunk(
            &orchestrator,
            &compiled,
            &mut run,
            &scan_thunk,
            json!({"items": [1, 2, 3]}),
        )
        .await;
        orchestrator.tick(&compiled, &mut run).await.unwrap();

        // for_each concurrency=2，前两个 shard 应已派发到 dispatcher 并写入 tracker.
        let shard0 = tracker.map_shard(&run.run_id, "loop", 0).await;
        let shard1 = tracker.map_shard(&run.run_id, "loop", 1).await;
        assert!(shard0.is_some(), "shard 0 view should be recorded");
        assert!(shard1.is_some(), "shard 1 view should be recorded");
        let shard0 = shard0.unwrap();
        assert_eq!(shard0.for_each_id, "loop");
        assert!(matches!(shard0.state, NodeRunState::Running));
    }
}
