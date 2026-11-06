use super::*;
use crate::compiler::compile_workflow_with;
use crate::dispatcher::InMemoryThunkDispatcher;
use crate::dsl::WorkflowDefinition;
use crate::function_registry::FunctionRegistry;
use crate::functions::test_functions::{self, Echo};
use crate::native::{ToolEnvironment, ToolInfo};
use crate::object_store::InMemoryObjectStore;
use crate::task_tracker::{NoopTaskTracker, RecordingTaskTracker};
use buckyos_api::ThunkExecutionResult;
use serde_json::json;
use std::collections::BTreeMap;

type TestOrchestrator<T> = WorkflowOrchestrator<InMemoryThunkDispatcher, InMemoryObjectStore, T>;

struct Harness<T> {
    orchestrator: TestOrchestrator<T>,
    dispatcher: Arc<InMemoryThunkDispatcher>,
    store: Arc<InMemoryObjectStore>,
    registry: Arc<FunctionRegistry>,
    echo: Arc<Echo>,
}

fn registry() -> (Arc<FunctionRegistry>, Arc<Echo>) {
    let mut registry = FunctionRegistry::new();
    let echo = Arc::new(Echo::default());
    test_functions::register_with_echo(&mut registry, echo.clone()).unwrap();
    (Arc::new(registry), echo)
}

fn harness_with<T: WorkflowTaskTracker + 'static>(tracker: Arc<T>) -> Harness<T> {
    let (registry, echo) = registry();
    let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
    let store = Arc::new(InMemoryObjectStore::new());
    let orchestrator = WorkflowOrchestrator::new(dispatcher.clone(), store.clone(), tracker)
        .with_functions(registry.clone());
    Harness {
        orchestrator,
        dispatcher,
        store,
        registry,
        echo,
    }
}

fn harness() -> Harness<NoopTaskTracker> {
    harness_with(Arc::new(NoopTaskTracker))
}

fn definition(value: Value) -> WorkflowDefinition {
    serde_json::from_value(value).unwrap()
}

fn compile(registry: &FunctionRegistry, value: Value) -> CompiledWorkflow {
    compile_workflow_with(definition(value), registry)
        .unwrap()
        .workflow
}

fn step(id: &str, executor: &str, input: Value, idempotent: bool, fallback: &str) -> Value {
    json!({
        "id": id,
        "name": id,
        "executor": executor,
        "type": "autonomous",
        "idempotent": idempotent,
        "skippable": false,
        "input": input,
        "output_schema": {"type": "object", "properties": {"echo": {}, "calls": {"type": "integer"}}},
        "guards": {"retry": {"max_attempts": 1, "fallback": fallback}}
    })
}

fn echo_chain(value: Value) -> Value {
    json!({
        "schema_version": "0.2.0",
        "id": "wf-echo",
        "name": "echo",
        "trigger": {"type": "manual"},
        "steps": [
            step("a", "/tool/test.echo@1", json!({"value": value}), true, "abort"),
            step("b", "/tool/test.echo@1", json!({"value": {"from_a": "${a.output.echo}"}}), true, "abort"),
        ],
        "edges": [{"from": "a", "to": "b"}, {"from": "b"}]
    })
}

fn success(execution: &ThunkExecution, result: Value) -> ThunkExecutionResult {
    ThunkExecutionResult::success(execution, result)
}

#[tokio::test]
async fn tool_links_bind_to_real_function_objects() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!(1)));
    let fun_id = h.registry.resolve("test.echo", "1").unwrap().fun_id().to_string();
    assert!(fun_id.starts_with("func:"));
    assert_eq!(compiled.resolved_functions["a"], fun_id);
    assert_eq!(compiled.resolved_functions["b"], fun_id);
    let Expr::Apply { executor, fun_id: bound, .. } = &compiled.nodes["a"].expr else {
        panic!("apply");
    };
    assert_eq!(executor.as_str(), format!("func::{fun_id}"));
    assert_eq!(bound.as_deref(), Some(fun_id.as_str()));
    assert_eq!(compiled.revision.len(), 64);
    let again = compile(&h.registry, echo_chain(json!(1)));
    assert_eq!(again.revision, compiled.revision);
    let changed = compile(&h.registry, echo_chain(json!(2)));
    assert_ne!(changed.revision, compiled.revision);

    let err = compile_workflow_with(definition(json!({
        "schema_version": "0.2.0", "id": "wf-x", "name": "x", "trigger": {"type": "manual"},
        "steps": [step("a", "/tool/test.missing@1", json!({}), true, "abort")],
        "edges": [{"from": "a"}]
    })), &h.registry).unwrap_err();
    assert!(matches!(err, WorkflowError::UnresolvedFunction { .. }));
}

#[tokio::test]
async fn thunk_round_trip_uses_execution_ids_and_canonical_thunk_objects() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!({"k": "v"})));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    let events = h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert!(events.iter().any(|e| e.event_type == "step.started"));

    let scheduled = h.dispatcher.scheduled().await;
    assert_eq!(scheduled.len(), 1);
    let execution = scheduled[0].execution.clone();
    assert_eq!(execution.execution_id, format!("{}/a/1", run.run_id));
    assert_eq!(execution.node_id, "a");
    assert!(execution.cache_key.is_some());
    assert_eq!(run.pending_thunks[&execution.execution_id].thunk_obj_id, execution.thunk_obj_id.to_string());

    // The thunk object is stored under its real id and reads back verified.
    let stored = crate::object_store::get_verified_json(h.store.as_ref(), &execution.thunk_obj_id.to_string())
        .await
        .unwrap()
        .expect("thunk stored");
    assert_eq!(stored["params"]["value"]["k"], "v");
    let thunk: ThunkObject = serde_json::from_value(stored).unwrap();
    assert_eq!(thunk.calc_obj_id().unwrap().0, execution.thunk_obj_id);

    // The function object is published and matches the bound fun_id.
    h.registry.publish(h.store.as_ref()).await.unwrap();
    let function = crate::object_store::get_verified_json(h.store.as_ref(), &thunk.fun_id.to_string())
        .await
        .unwrap()
        .expect("function published");
    assert_eq!(function["func_type"], "native");

    let events = h
        .orchestrator
        .handle_thunk_result(&compiled, &mut run, success(&execution, json!({"echo": {"k": "v"}, "calls": 1})))
        .await
        .unwrap();
    assert!(events.iter().any(|e| e.event_type == "step.completed"));
    assert_eq!(run.node_states["a"], NodeRunState::Completed);
    assert_eq!(run.node_sources["a"], "executed");
    assert!(run.pending_thunks.is_empty());

    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let scheduled = h.dispatcher.scheduled().await;
    assert_eq!(scheduled.len(), 2);
    assert_eq!(scheduled[1].execution.node_id, "b");
    assert_eq!(scheduled[1].thunk.params["value"], json!({"from_a": {"k": "v"}}));
    let b = scheduled[1].execution.clone();
    h.orchestrator
        .handle_thunk_result(&compiled, &mut run, success(&b, json!({"echo": {"k": "v"}, "calls": 2})))
        .await
        .unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.output["b"]["echo"]["k"], "v");
}

#[tokio::test]
async fn duplicate_late_and_mismatched_receipts_are_rejected() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!("x")));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let execution = h.dispatcher.scheduled().await[0].execution.clone();

    // Receipt for another thunk under the same execution id.
    let mut forged = success(&execution, json!({"echo": "x"}));
    forged.thunk_obj_id = ObjId::new("thunk:00").unwrap();
    assert!(matches!(
        h.orchestrator.handle_thunk_result(&compiled, &mut run, forged).await,
        Err(WorkflowError::StaleThunkResult(_))
    ));
    assert_eq!(run.node_states["a"], NodeRunState::Running);

    h.orchestrator
        .handle_thunk_result(&compiled, &mut run, success(&execution, json!({"echo": "x"})))
        .await
        .unwrap();
    let seq_after_first = run.seq;
    // Duplicate.
    assert!(matches!(
        h.orchestrator
            .handle_thunk_result(&compiled, &mut run, success(&execution, json!({"echo": "dup"})))
            .await,
        Err(WorkflowError::StaleThunkResult(_))
    ));
    assert_eq!(run.seq, seq_after_first);
    assert_eq!(run.node_outputs["a"]["echo"], "x");

    // Late receipt after the run is terminal.
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let b = h.dispatcher.scheduled().await[1].execution.clone();
    h.orchestrator
        .handle_thunk_result(&compiled, &mut run, success(&b, json!({"echo": "x"})))
        .await
        .unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert!(matches!(
        h.orchestrator
            .handle_thunk_result(&compiled, &mut run, success(&b, json!({"echo": "late"})))
            .await,
        Err(WorkflowError::StaleThunkResult(_))
    ));
    assert_eq!(run.output["b"]["echo"], "x");
}

#[tokio::test]
async fn same_computation_has_same_thunk_id_across_runs_but_distinct_executions() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!("same")));
    let (mut run1, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    let (mut run2, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run1).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run2).await.unwrap();
    let scheduled = h.dispatcher.scheduled().await;
    assert_eq!(scheduled.len(), 2);
    assert_eq!(scheduled[0].execution.thunk_obj_id, scheduled[1].execution.thunk_obj_id);
    assert_ne!(scheduled[0].execution.execution_id, scheduled[1].execution.execution_id);
    // Receipts never cross runs.
    let other = scheduled[1].execution.clone();
    assert!(matches!(
        h.orchestrator.handle_thunk_result(&compiled, &mut run1, success(&other, json!({"echo": 1}))).await,
        Err(WorkflowError::StaleThunkResult(_))
    ));
}

#[tokio::test]
async fn pure_step_results_are_reused_from_the_cache_after_validation() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!("cached")));
    let (mut run1, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run1).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    let mut receipt = success(&a, json!({"echo": "cached", "calls": 1}));
    receipt.artifact_refs = vec![h.store.put_json("artifact", &json!({"page": 1})).await.unwrap()];
    receipt.fingerprint = BTreeMap::from([("cmd/x".to_string(), "1".to_string())]);
    h.orchestrator.handle_thunk_result(&compiled, &mut run1, receipt).await.unwrap();
    assert_eq!(h.orchestrator.cache.len().await.unwrap(), 1);

    let (mut run2, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    let events = h.orchestrator.tick(&compiled, &mut run2).await.unwrap();
    assert_eq!(run2.node_states["a"], NodeRunState::Completed);
    assert_eq!(run2.node_sources["a"], "cache");
    assert_eq!(run2.node_outputs["a"]["echo"], "cached");
    assert!(events.iter().any(|e| e.event_type == "step.completed" && e.payload.as_ref().unwrap()["source"] == "cache"));
    // Step b depends on a's output and is scheduled normally.
    let scheduled = h.dispatcher.scheduled().await;
    assert_eq!(scheduled.len(), 2);
    assert_eq!(scheduled[1].execution.node_id, "b");

    // A missing artifact invalidates the entry: the step is recomputed.
    let key = a.cache_key.clone().unwrap();
    let mut broken = h.orchestrator.cache.get(&key).await.unwrap().unwrap();
    broken.artifact_refs.push("cyfile:deadbeef".to_string());
    h.orchestrator.cache.put(&key, &broken).await.unwrap();
    let (mut run3, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run3).await.unwrap();
    assert_eq!(run3.node_states["a"], NodeRunState::Running);
    assert!(h.orchestrator.cache.get(&key).await.unwrap().is_none());

    // Expired entries are dropped as well.
    let a3 = h.dispatcher.scheduled().await.last().unwrap().execution.clone();
    h.orchestrator.handle_thunk_result(&compiled, &mut run3, success(&a3, json!({"echo": "cached"}))).await.unwrap();
    let mut expired = h.orchestrator.cache.get(&key).await.unwrap().unwrap();
    expired.expires_at = Some(1);
    h.orchestrator.cache.put(&key, &expired).await.unwrap();
    let (mut run4, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run4).await.unwrap();
    assert_eq!(run4.node_states["a"], NodeRunState::Running);
}

#[tokio::test]
async fn non_cacheable_results_and_impure_functions_never_touch_the_cache() {
    let h = harness();
    // Partial result: cacheable=false.
    let compiled = compile(&h.registry, echo_chain(json!("partial")));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    let mut receipt = success(&a, json!({"echo": "partial"}));
    receipt.cacheable = false;
    h.orchestrator.handle_thunk_result(&compiled, &mut run, receipt).await.unwrap();
    assert_eq!(h.orchestrator.cache.len().await.unwrap(), 0);

    // Impure function (test.sleep is registered with is_pure=false): no cache key.
    let sleepy = json!({
        "schema_version": "0.2.0", "id": "wf-sleep", "name": "sleep", "trigger": {"type": "manual"},
        "steps": [step("s", "/tool/test.sleep@1", json!({"ms": 1}), true, "abort")],
        "edges": [{"from": "s"}]
    });
    let compiled = compile(&h.registry, sleepy);
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let s = h.dispatcher.scheduled().await.last().unwrap().execution.clone();
    assert!(s.cache_key.is_none());
    h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&s, json!({"slept_ms": 1}))).await.unwrap();
    assert_eq!(h.orchestrator.cache.len().await.unwrap(), 0);

    // Non-idempotent step with a pure function: not cached either.
    let compiled = compile(&h.registry, json!({
        "schema_version": "0.2.0", "id": "wf-ni", "name": "ni", "trigger": {"type": "manual"},
        "steps": [step("a", "/tool/test.echo@1", json!({"value": 1}), false, "abort")],
        "edges": [{"from": "a"}]
    }));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert!(h.dispatcher.scheduled().await.last().unwrap().execution.cache_key.is_none());
}

#[tokio::test]
async fn fingerprint_changes_miss_the_cache_but_unrelated_tools_do_not() {
    let (registry, _) = registry();
    let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
    let store = Arc::new(InMemoryObjectStore::new());
    let compiled = compile(&registry, echo_chain(json!("fp")));
    let env = |pdftoppm: &str, ffmpeg: &str| {
        Arc::new(ToolEnvironment::with_tools(BTreeMap::from([
            ("pdftoppm".to_string(), ToolInfo { path: "/bin/pdftoppm".into(), version: pdftoppm.to_string() }),
            ("ffmpeg".to_string(), ToolInfo { path: "/bin/ffmpeg".into(), version: ffmpeg.to_string() }),
        ])))
    };
    let keys = |e: Arc<ToolEnvironment>| {
        let orchestrator = WorkflowOrchestrator::new(dispatcher.clone(), store.clone(), Arc::new(NoopTaskTracker))
            .with_functions(registry.clone())
            .with_environment(Arc::new(StaticEnvironment(e)));
        let compiled = compiled.clone();
        let dispatcher = dispatcher.clone();
        async move {
            let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
            orchestrator.tick(&compiled, &mut run).await.unwrap();
            dispatcher.scheduled().await.last().unwrap().execution.clone()
        }
    };
    let base = keys(env("24.02", "7.1")).await;
    let other_tool = keys(env("24.02", "6.0")).await;
    // test.echo depends on no tools, so its fingerprint is empty and keys agree.
    assert_eq!(base.cache_key, other_tool.cache_key);
    assert!(base.fingerprint.is_empty());
    // Direct check of the key rule with a dependent fingerprint.
    let fp_a = BTreeMap::from([("cmd/pdftoppm".to_string(), "24.02".to_string())]);
    let fp_b = BTreeMap::from([("cmd/pdftoppm".to_string(), "22.12".to_string())]);
    let id = base.thunk_obj_id.to_string();
    assert_ne!(step_cache_key(&id, &fp_a), step_cache_key(&id, &fp_b));
    assert_eq!(step_cache_key(&id, &fp_a), step_cache_key(&id, &fp_a));
}

#[tokio::test]
async fn failure_with_abort_fallback_fails_the_run_with_structured_error() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!("fail")));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    let error = ThunkExecutionError::new(error_codes::TOOLS_MISSING, "pdftoppm missing")
        .with_details(json!({"tools_missing": ["pdftoppm"]}));
    let events = h
        .orchestrator
        .handle_thunk_result(&compiled, &mut run, ThunkExecutionResult::failed(&a, error))
        .await
        .unwrap();
    assert_eq!(run.node_states["a"], NodeRunState::Failed);
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.node_errors["a"]["code"], "tools_missing");
    assert_eq!(run.node_errors["a"]["details"]["tools_missing"][0], "pdftoppm");
    assert!(run.human_waiting_nodes.is_empty());
    assert!(events.iter().any(|e| e.event_type == "step.failed" && e.payload.as_ref().unwrap()["error"]["code"] == "tools_missing"));
    assert!(events.iter().any(|e| e.event_type == "run.failed"));
}

#[tokio::test]
async fn failure_with_human_fallback_waits_for_human_and_retry_resumes() {
    let h = harness();
    let compiled = compile(&h.registry, json!({
        "schema_version": "0.2.0", "id": "wf-h", "name": "h", "trigger": {"type": "manual"},
        "steps": [step("a", "/tool/test.echo@1", json!({"value": 1}), true, "human")],
        "edges": [{"from": "a"}]
    }));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    h.orchestrator
        .handle_thunk_result(&compiled, &mut run, ThunkExecutionResult::failed(&a, ThunkExecutionError::new("tool_failed", "boom")))
        .await
        .unwrap();
    assert_eq!(run.status, RunStatus::WaitingHuman);
    h.orchestrator
        .handle_human_action(&compiled, &mut run, HumanAction { node_id: "a".into(), action: HumanActionKind::Retry, payload: None, actor: "human".into() })
        .await
        .unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let scheduled = h.dispatcher.scheduled().await;
    assert_eq!(scheduled.len(), 2);
    assert_eq!(scheduled[1].execution.attempt, 2);
    assert_eq!(scheduled[1].execution.execution_id, format!("{}/a/2", run.run_id));
    assert_eq!(scheduled[1].execution.thunk_obj_id, a.thunk_obj_id);
}

#[tokio::test]
async fn retry_policy_retries_before_failing() {
    let h = harness();
    let mut def = echo_chain(json!("retry"));
    def["steps"][0]["guards"]["retry"]["max_attempts"] = json!(2);
    let compiled = compile(&h.registry, def);
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a1 = h.dispatcher.scheduled().await[0].execution.clone();
    h.orchestrator
        .handle_thunk_result(&compiled, &mut run, ThunkExecutionResult::failed(&a1, ThunkExecutionError::new("tool_failed", "boom").retryable()))
        .await
        .unwrap();
    assert_eq!(run.node_states["a"], NodeRunState::Pending);
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a2 = h.dispatcher.scheduled().await[1].execution.clone();
    assert_eq!(a2.attempt, 2);
    h.orchestrator
        .handle_thunk_result(&compiled, &mut run, ThunkExecutionResult::failed(&a2, ThunkExecutionError::new("tool_failed", "boom")))
        .await
        .unwrap();
    assert_eq!(run.node_states["a"], NodeRunState::Failed);
    assert_eq!(run.status, RunStatus::Failed);
}

#[tokio::test]
async fn cancel_run_cancels_pending_executions_and_aborts() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!("cancel")));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    let events = h.orchestrator.cancel_run(&compiled, &mut run, "u/app", None).await.unwrap();
    assert_eq!(run.status, RunStatus::Aborted);
    assert_eq!(h.dispatcher.cancelled().await, vec![a.execution_id.clone()]);
    assert!(run.pending_thunks.is_empty());
    assert_eq!(run.node_states["a"], NodeRunState::Aborted);
    assert_eq!(run.node_errors["a"]["code"], "cancelled");
    assert!(events.iter().any(|e| e.event_type == "run.aborted"));
    assert!(matches!(h.orchestrator.cancel_run(&compiled, &mut run, "u/app", None).await, Err(WorkflowError::RunTerminal(_))));
    assert!(matches!(
        h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&a, json!({"echo": 1}))).await,
        Err(WorkflowError::StaleThunkResult(_))
    ));
}

#[tokio::test]
async fn run_deadline_aborts_on_tick() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!("deadline")));
    let (mut run, _) = h
        .orchestrator
        .create_run_with_options(&compiled, BTreeMap::new(), RunOptions { deadline_at: Some(1), ..Default::default() })
        .await
        .unwrap();
    let events = h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(events.iter().any(|e| e.event_type == "run.failed"));
    assert_eq!(run.metrics["run_error"]["code"], "timeout");
    assert!(h.dispatcher.scheduled().await.is_empty());
}

#[tokio::test]
async fn execution_deadline_is_the_earliest_of_run_step_and_function_limits() {
    let h = harness();
    let mut def = echo_chain(json!("dl"));
    def["steps"][0]["guards"]["timeout"] = json!("5s");
    def["guards"] = json!({"max_duration": "10m"});
    let compiled = compile(&h.registry, def);
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    let now = Utc::now().timestamp();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    let deadline = a.deadline_at.unwrap();
    assert!(deadline >= now + 4 && deadline <= now + 6, "deadline {deadline} now {now}");
    assert!(run.deadline_at.unwrap() >= now + 599);
    assert_eq!(a.registration.as_ref().unwrap().timeout, Some(30));
    assert_eq!(parse_duration_secs("2m"), Some(120));
    assert_eq!(parse_duration_secs("90"), Some(90));
    assert_eq!(parse_duration_secs("1h"), Some(3600));
    assert_eq!(parse_duration_secs("abc"), None);
}

#[tokio::test]
async fn trigger_output_feeds_steps_and_run_meta() {
    let h = harness();
    let compiled = compile(&h.registry, json!({
        "schema_version": "0.2.0", "id": "wf-t", "name": "t",
        "trigger": {"type": "manual", "input_schema": {"type": "object", "properties": {"obj_id": {"type": "string"}, "content_id": {"type": "string"}}}},
        "steps": [step("a", "/tool/test.echo@1", json!({"value": "${trigger.output.content_id}"}), true, "abort")],
        "edges": [{"from": "a"}]
    }));
    let (mut run, _) = h
        .orchestrator
        .create_run_with_options(&compiled, BTreeMap::new(), RunOptions {
            requested_by: Some("alice/agent".into()),
            execution_epoch: 7,
            trigger_output: Some(json!({"obj_id": "cyfile:ab", "content_id": "sha256:cd"})),
            deadline_at: None,
        })
        .await
        .unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    assert_eq!(h.dispatcher.scheduled().await[0].thunk.params["value"], "sha256:cd");
    assert_eq!(a.requested_by.as_deref(), Some("alice/agent"));
    assert_eq!(a.source_obj_id.as_deref(), Some("cyfile:ab"));
    assert_eq!(a.execution_epoch, 7);
    assert_eq!(run.definition_revision, compiled.revision);
}

#[tokio::test]
async fn parallel_join_all_and_for_each_shards_use_execution_ids() {
    let h = harness();
    let compiled = compile(&h.registry, json!({
        "schema_version": "0.2.0", "id": "wf-par", "name": "par", "trigger": {"type": "manual"},
        "steps": [
            {"id": "seed", "name": "seed", "executor": "/tool/test.echo@1", "type": "autonomous", "idempotent": true, "skippable": false,
             "input": {"value": {"items": [1, 2, 3]}}, "output_mode": "finite_seekable",
             "output_schema": {"type": "object", "properties": {"echo": {"type": "object", "properties": {"items": {"type": "array", "items": {"type": "integer"}}}}, "element_schema": {"type": "object"}, "calls": {"type": "integer"}}},
             "guards": {"retry": {"max_attempts": 1, "fallback": "abort"}}},
            step("x", "/tool/test.echo@1", json!({"value": "x"}), true, "abort"),
            step("y", "/tool/test.echo@1", json!({"value": "y"}), true, "abort"),
            step("body", "/tool/test.echo@1", json!({"value": "item"}), true, "abort"),
        ],
        "nodes": [
            {"type": "parallel", "id": "par", "branches": ["x", "y"], "join": "all"},
            {"type": "for_each", "id": "loop", "items": "${seed.output.echo}", "steps": ["body"], "max_items": 10, "concurrency": 2}
        ],
        "edges": [{"from": "seed", "to": "par"}, {"from": "par", "to": "loop"}, {"from": "loop"}]
    }));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let seed = h.dispatcher.scheduled().await[0].execution.clone();
    h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&seed, json!({"echo": {"items": [1, 2, 3]}}))).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let scheduled = h.dispatcher.scheduled().await;
    assert_eq!(scheduled.len(), 3);
    for s in &scheduled[1..] {
        h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&s.execution, json!({"echo": s.execution.node_id.clone()}))).await.unwrap();
    }
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert_eq!(run.node_states["par"], NodeRunState::Completed);
    assert_eq!(run.node_outputs["par"]["x"]["echo"], "x");
    // for_each: 3 items, concurrency 2.
    let scheduled = h.dispatcher.scheduled().await;
    assert_eq!(scheduled.len(), 5);
    assert_eq!(scheduled[3].execution.node_id, "loop[0]");
    assert_eq!(scheduled[3].execution.execution_id, format!("{}/loop[0]/1", run.run_id));
    assert_eq!(scheduled[4].execution.node_id, "loop[1]");
    assert_ne!(scheduled[3].execution.thunk_obj_id, scheduled[4].execution.thunk_obj_id);
    let s0 = scheduled[3].execution.clone();
    h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&s0, json!({"echo": "i0"}))).await.unwrap();
    let scheduled = h.dispatcher.scheduled().await;
    assert_eq!(scheduled.len(), 6);
    assert_eq!(scheduled[5].execution.node_id, "loop[2]");
    for s in [scheduled[4].execution.clone(), scheduled[5].execution.clone()] {
        h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&s, json!({"echo": s.node_id.clone()}))).await.unwrap();
    }
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.output["loop"].as_array().unwrap().len(), 3);
}

fn echo_registry() -> Arc<crate::executor_adapter::ExecutorRegistry> {
    let adapter = crate::executor_adapter::NamespaceAdapter::new(["service", "http"], |executor, input| {
        let executor = executor.clone();
        let input = input.clone();
        Box::pin(async move {
            if executor.as_str().ends_with("fail") {
                return Err(WorkflowError::Dispatcher("adapter boom".into()));
            }
            Ok(json!({"executor": executor.as_str(), "input": input}))
        })
    });
    Arc::new(crate::executor_adapter::ExecutorRegistry::new().with(Arc::new(adapter)))
}

#[tokio::test]
async fn direct_adapters_run_inline_and_cache_by_executor() {
    let h = harness();
    let orchestrator = WorkflowOrchestrator::new(h.dispatcher.clone(), h.store.clone(), Arc::new(NoopTaskTracker))
        .with_functions(h.registry.clone())
        .with_executor_registry(echo_registry());
    let compiled = compile(&h.registry, json!({
        "schema_version": "0.2.0", "id": "wf-svc", "name": "svc", "trigger": {"type": "manual"},
        "steps": [
            {"id": "a", "name": "a", "executor": "service::demo.scan", "type": "autonomous", "idempotent": true, "skippable": false,
             "input": {"q": 1}, "output_schema": {"type": "object"}},
            {"id": "b", "name": "b", "executor": "http::demo.fail", "type": "autonomous", "idempotent": false, "skippable": false,
             "input": {}, "output_schema": {"type": "object"}, "guards": {"retry": {"max_attempts": 1, "fallback": "abort"}}}
        ],
        "edges": [{"from": "a", "to": "b"}, {"from": "b"}]
    }));
    let (mut run, _) = orchestrator.create_run(&compiled).await.unwrap();
    orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert_eq!(run.node_sources["a"], "direct");
    assert_eq!(run.node_outputs["a"]["input"]["q"], 1);
    assert_eq!(run.node_states["b"], NodeRunState::Failed);
    assert_eq!(run.node_errors["b"]["code"], "executor_error");
    assert_eq!(run.status, RunStatus::Failed);
    assert!(h.dispatcher.scheduled().await.is_empty());

    let (mut run2, _) = orchestrator.create_run(&compiled).await.unwrap();
    orchestrator.tick(&compiled, &mut run2).await.unwrap();
    assert_eq!(run2.node_sources["a"], "cache");
}

#[tokio::test]
async fn human_confirm_and_branch_flow_still_work() {
    let h = harness();
    let compiled = compile(&h.registry, json!({
        "schema_version": "0.2.0", "id": "wf-human", "name": "human", "trigger": {"type": "manual"},
        "steps": [
            {"id": "plan", "name": "plan", "executor": "/tool/test.echo@1", "type": "autonomous", "idempotent": true, "skippable": false,
             "input": {"value": "draft"}, "output_schema": {"type": "object", "properties": {"echo": {"type": "string"}}, "required": ["echo"]}},
            {"id": "review", "name": "review", "type": "human_confirm", "skippable": false, "subject_ref": "${plan.output}", "prompt": "ok?",
             "output_schema": {"type": "object", "properties": {"decision": {"type": "string", "enum": ["approved", "rejected"]}, "final_subject": {"type": "object", "properties": {"echo": {"type": "string"}}, "required": ["echo"]}}, "required": ["decision", "final_subject"]}},
            {"id": "publish", "name": "publish", "executor": "/tool/test.echo@1", "type": "autonomous", "idempotent": true, "skippable": true,
             "input": {"value": "${review.output.final_subject}"}, "output_schema": {"type": "object"}}
        ],
        "nodes": [{"type": "branch", "id": "decision", "on": "${review.output.decision}", "paths": {"approved": "publish", "rejected": "publish"}, "max_iterations": 2}],
        "edges": [{"from": "plan", "to": "review"}, {"from": "review", "to": "decision"}, {"from": "publish"}]
    }));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let plan = h.dispatcher.scheduled().await[0].execution.clone();
    h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&plan, json!({"echo": "draft"}))).await.unwrap();
    let events = h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert!(events.iter().any(|e| e.event_type == "step.waiting_human"));
    assert_eq!(run.status, RunStatus::WaitingHuman);

    h.orchestrator
        .handle_human_action(&compiled, &mut run, HumanAction { node_id: "review".into(), action: HumanActionKind::Approve, payload: None, actor: "human/test".into() })
        .await
        .unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert_eq!(run.node_outputs["decision"]["branch"], "approved");
    assert_eq!(run.node_states["publish"], NodeRunState::Running);
    let publish = h.dispatcher.scheduled().await[1].execution.clone();
    assert_eq!(h.dispatcher.scheduled().await[1].thunk.params["value"]["echo"], "draft");
    h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&publish, json!({"echo": 1}))).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);
}

#[tokio::test]
async fn submit_step_output_cancels_pending_execution() {
    let h = harness();
    let compiled = compile(&h.registry, echo_chain(json!("agent")));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    h.orchestrator
        .submit_step_output(&compiled, &mut run, "a", "agent", json!({"echo": "manual"}))
        .await
        .unwrap();
    assert_eq!(h.dispatcher.cancelled().await, vec![a.execution_id.clone()]);
    assert_eq!(run.node_sources["a"], "submitted");
    assert!(matches!(
        h.orchestrator.handle_thunk_result(&compiled, &mut run, success(&a, json!({"echo": "late"}))).await,
        Err(WorkflowError::StaleThunkResult(_))
    ));
    assert_eq!(run.node_outputs["a"]["echo"], "manual");
}

#[tokio::test]
async fn tracker_sees_execution_keyed_thunks_and_structured_step_errors() {
    let tracker = Arc::new(RecordingTaskTracker::new());
    let h = harness_with(tracker.clone());
    let compiled = compile(&h.registry, echo_chain(json!("track")));
    let (mut run, _) = h.orchestrator.create_run(&compiled).await.unwrap();
    h.orchestrator.tick(&compiled, &mut run).await.unwrap();
    let a = h.dispatcher.scheduled().await[0].execution.clone();
    let view = tracker.thunk(&run.run_id, &a.execution_id).await.expect("thunk task view");
    assert_eq!(view.thunk_obj_id, a.thunk_obj_id.to_string());
    assert_eq!(view.attempt, 1);
    h.orchestrator
        .handle_thunk_result(&compiled, &mut run, ThunkExecutionResult::failed(&a, ThunkExecutionError::new("timeout", "too slow")))
        .await
        .unwrap();
    let step = tracker.step(&run.run_id, "a").await.unwrap();
    assert_eq!(step.state, NodeRunState::Failed);
    assert_eq!(step.error_detail.as_ref().unwrap()["code"], "timeout");
    assert_eq!(step.error.as_deref(), Some("timeout: too slow"));
    let _ = h.echo.calls();
}
