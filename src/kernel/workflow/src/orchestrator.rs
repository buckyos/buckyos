use crate::compiler::{CompiledNode, CompiledWorkflow};
use crate::dispatcher::ThunkDispatcher;
use crate::dsl::RetryFallback;
use crate::error::{WorkflowError, WorkflowResult};
use crate::object_store::{deterministic_object_id, WorkflowObjectStore};
use crate::runtime::{
    EventEnvelope, HumanAction, HumanActionKind, HumanWait, MapState, NodeRunState, ParJoin,
    ParState, PendingThunk, RunStatus, WorkflowRun,
};
use crate::task_tracker::WorkflowTaskTracker;
use crate::types::{AwaitKind, Expr, JoinStrategy, RetryPolicy, ValueTemplate};
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
        }
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
                        self.enter_human_wait(workflow, run, compiled, &mut events)?;
                    }
                    Expr::Par { .. } => {
                        self.enter_par(workflow, run, compiled, &mut events)?;
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
                        Some(node_id),
                        "engine",
                        Some(json!({ "result_obj_id": output_id })),
                    ));
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
                if attempt < policy.max_attempts && result.status == ThunkExecutionStatus::Failed {
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::Retrying);
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::Pending);
                    events.push(self.emit_event(
                        run,
                        "step.retrying",
                        Some(node_id),
                        "engine",
                        Some(json!({ "attempt": attempt + 1, "error": result.error })),
                    ));
                } else if policy.fallback == RetryFallback::Human {
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::WaitingHuman);
                    run.human_waiting_nodes.insert(node_id.clone());
                    events.push(self.emit_event(
                        run,
                        "step.waiting_human",
                        Some(node_id),
                        "engine",
                        Some(json!({ "reason": "retries_exhausted", "error": result.error })),
                    ));
                } else {
                    run.node_states
                        .insert(node_id.clone(), NodeRunState::Failed);
                    events.push(self.emit_event(
                        run,
                        "step.failed",
                        Some(node_id),
                        "engine",
                        Some(json!({ "error": result.error })),
                    ));
                }
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
            Some(action.node_id),
            action.actor.as_str(),
            Some(json!({ "action": action.action.as_str(), "payload": action.payload })),
        ));
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
                events.push(self.emit_event(
                    run,
                    "step.completed",
                    Some(compiled.id.clone()),
                    "engine",
                    Some(json!({ "source": "cache" })),
                ));
                return Ok(());
            }
        }

        let next_attempt = {
            let attempt = run.node_attempts.entry(compiled.id.clone()).or_insert(0);
            *attempt += 1;
            *attempt
        };
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
                thunk_obj_id,
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
        Ok(())
    }

    fn enter_human_wait(
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
        let wait = HumanWait {
            node_id: compiled.id.clone(),
            prompt: prompt.clone(),
            subject,
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

    fn enter_par(
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

        if total == 0 {
            self.complete_map(workflow, run, &compiled.id, events).await?;
            return Ok(());
        }

        self.dispatch_pending_shards(run, &compiled.id, body_node).await?;
        Ok(())
    }

    async fn dispatch_pending_shards(
        &self,
        run: &mut WorkflowRun,
        for_each_id: &str,
        body_node: &CompiledNode,
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

        for index in 0..total {
            if running_count >= max_concurrency {
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
            running_count += 1;
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

        let body_step_id = run
            .map_states
            .get(for_each_id)
            .map(|state| state.body_step_id.clone())
            .ok_or_else(|| WorkflowError::MissingMapState(for_each_id.to_string()))?;
        let body_node = workflow
            .nodes
            .get(&body_step_id)
            .ok_or_else(|| WorkflowError::NodeNotFound(body_step_id.clone()))?;
        self.dispatch_pending_shards(run, for_each_id, body_node)
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
        let _ = workflow;
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
        Ok(())
    }
}

fn build_thunk(
    run: &WorkflowRun,
    compiled: &CompiledNode,
    resolved_input: Value,
    attempt: u32,
) -> WorkflowResult<(String, ThunkObject)> {
    let Expr::Apply {
        fun_id,
        idempotent: _,
        guards: _,
        ..
    } = &compiled.expr
    else {
        return Err(WorkflowError::UnsupportedNode(compiled.id.clone()));
    };
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
    let Expr::Apply { fun_id, .. } = &body_node.expr else {
        return Err(WorkflowError::ForEachBodyNotApply(
            for_each_id.to_string(),
            body_step_id.to_string(),
        ));
    };
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
    let Expr::Apply { fun_id, .. } = &compiled.expr else {
        return String::new();
    };
    let payload = json!({
        "fun_id": fun_id,
        "input": resolved_input,
    });
    deterministic_object_id("workflow_cache", &payload).unwrap_or_default()
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
                    executor: Some("agent/mia".to_string()),
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
                    executor: Some("skill/finalize".to_string()),
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
                    executor: Some("skill/seed".to_string()),
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
                    executor: Some("skill/a".to_string()),
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
                    executor: Some("skill/b".to_string()),
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
                    executor: Some("skill/join".to_string()),
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

    async fn finish_thunk(
        orchestrator: &WorkflowOrchestrator<
            InMemoryThunkDispatcher,
            InMemoryObjectStore,
            NoopTaskTracker,
        >,
        compiled: &CompiledWorkflow,
        run: &mut WorkflowRun,
        thunk_id: &str,
        result: serde_json::Value,
    ) {
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
                    executor: Some("skill/fs".to_string()),
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
                    executor: Some("skill/ingest".to_string()),
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
                    executor: Some("skill/summary".to_string()),
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
}
