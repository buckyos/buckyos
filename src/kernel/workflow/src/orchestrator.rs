use crate::compiler::{CompiledNode, CompiledWorkflow};
use crate::dispatcher::ThunkDispatcher;
use crate::dsl::RetryFallback;
use crate::error::{WorkflowError, WorkflowResult};
use crate::object_store::{deterministic_object_id, WorkflowObjectStore};
use crate::runtime::{
    EventEnvelope, HumanAction, HumanActionKind, HumanWait, NodeRunState, PendingThunk, RunStatus,
    WorkflowRun,
};
use crate::task_tracker::WorkflowTaskTracker;
use crate::types::{AwaitKind, Expr, RetryPolicy, ValueTemplate};
use buckyos_api::{ThunkExecutionResult, ThunkExecutionStatus, ThunkObject};
use chrono::Utc;
use ndn_lib::ObjId;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
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
                    Expr::Par { .. } | Expr::Map { .. } => {
                        return Err(WorkflowError::UnsupportedNode(node_id));
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

                run.node_states
                    .insert(node_id.clone(), NodeRunState::Completed);
                run.node_outputs.insert(node_id.clone(), output.clone());
                run.metrics.insert(node_id.clone(), result.metrics.clone());
                if is_idempotent(compiled) {
                    let cache_key = cache_key(compiled, &self.resolve_apply_input(run, compiled)?);
                    self.object_store
                        .put_json(
                            "workflow_cache",
                            &json!({
                                "result_obj_id": output_id,
                                "result": output,
                                "metrics": result.metrics,
                            }),
                        )
                        .await?;
                    let _ = cache_key;
                }
                self.activate_successors(workflow, run, &node_id);
                events.push(self.emit_event(
                    run,
                    "step.completed",
                    Some(node_id),
                    "engine",
                    Some(json!({ "result_obj_id": output_id })),
                ));
            }
            ThunkExecutionStatus::Failed | ThunkExecutionStatus::Cancelled => {
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
                .all(|dep| dependency_satisfied(run, dep))
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

    Ok((thunk_obj_id, ThunkObject {
        fun_id: ObjId::new(format!("func:{}", fun_id).as_str())
            .map_err(|err| WorkflowError::Serialization(format!("invalid fun_id `{}`: {}", fun_id, err)))?,
        params,
        metadata,
    }))
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

fn dependency_satisfied(run: &WorkflowRun, dependency: &str) -> bool {
    matches!(
        run.node_states.get(dependency),
        Some(NodeRunState::Completed | NodeRunState::Skipped)
    )
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
}
