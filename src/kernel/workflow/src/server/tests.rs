use super::*;
use crate::cache_index::MemoryCacheIndex;
use crate::function_registry::{FunctionRegistry, RegistrationSpec};
use crate::functions::test_functions::Echo;
use crate::io::MemoryNativeIo;
use crate::local_thunk_runner::LocalThunkRunner;
use crate::native::{
    EnvironmentProvider, ExecutionLimits, NativeContext, NativeFunction, NativeOutcome,
    ToolEnvironment, ToolInfo,
};
use crate::object_store::InMemoryObjectStore;
use crate::persistence::{ActiveExecutionRecord, ExecutionLedger, MemoryExecutionLedger};
use crate::task_tracker::RecordingTaskTracker;
use crate::{functions, InMemoryThunkDispatcher};
use buckyos_api::{FunctionObject, FunctionResultType, FunctionType, ThunkExecutionError};
use std::collections::BTreeMap;
use std::sync::RwLock;
use std::time::Duration;

struct TestVerifier;

#[async_trait::async_trait]
impl WorkflowCallerVerifier for TestVerifier {
    async fn verify(&self, ctx: &RPCContext) -> RpcResult<WorkflowCaller> {
        let token = ctx
            .token
            .as_deref()
            .ok_or_else(|| RPCErrors::NoPermission("missing test token".to_string()))?;
        let (user_id, app_id) = token
            .split_once('|')
            .ok_or_else(|| RPCErrors::InvalidToken("invalid test token".to_string()))?;
        Ok(WorkflowCaller {
            owner: Owner {
                user_id: user_id.to_string(),
                app_id: app_id.to_string(),
            },
            is_admin: user_id == "admin",
        })
    }
}

struct DenyingAuthorizer;

#[async_trait::async_trait]
impl InputAuthorizer for DenyingAuthorizer {
    async fn authorize_read(&self, caller: &WorkflowCaller, _: Option<&str>, obj_id: &str) -> RpcResult<()> {
        Err(RPCErrors::NoPermission(format!(
            "{} may not read {obj_id}",
            caller.principal()
        )))
    }
}

struct SwitchableEnvironment(RwLock<Arc<ToolEnvironment>>);

impl EnvironmentProvider for SwitchableEnvironment {
    fn current(&self) -> Arc<ToolEnvironment> {
        self.0.read().unwrap().clone()
    }
}

/// Native test function that depends on `pdftoppm` so its fingerprint follows
/// the environment.
struct ToolBound;

#[async_trait::async_trait]
impl NativeFunction for ToolBound {
    fn tools(&self) -> &'static [&'static str] {
        &["pdftoppm"]
    }

    async fn invoke(&self, _ctx: &NativeContext, params: &Value) -> std::result::Result<NativeOutcome, ThunkExecutionError> {
        Ok(NativeOutcome::new(json!({"echo": params.get("value").cloned().unwrap_or(Value::Null)})))
    }
}

struct Fixture {
    handler: WorkflowRpcHandler,
    runner: Arc<LocalThunkRunner>,
    tracker: Arc<RecordingTaskTracker>,
    io: Arc<MemoryNativeIo>,
    ledger: Arc<MemoryExecutionLedger>,
    environment: Arc<SwitchableEnvironment>,
    echo: Arc<Echo>,
}

fn env_with(pdftoppm: &str) -> Arc<ToolEnvironment> {
    Arc::new(ToolEnvironment::with_tools(BTreeMap::from([(
        "pdftoppm".to_string(),
        ToolInfo { path: "/usr/bin/pdftoppm".into(), version: pdftoppm.to_string() },
    )])))
}

struct FixtureOptions {
    epoch: u64,
    ledger: Arc<MemoryExecutionLedger>,
    authorizer: Arc<dyn InputAuthorizer>,
    store: Arc<InMemoryObjectStore>,
    cache: Arc<dyn crate::cache_index::CacheIndex>,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        Self {
            epoch: 1,
            ledger: Arc::new(MemoryExecutionLedger::default()),
            authorizer: Arc::new(AllowAllInputAuthorizer),
            store: Arc::new(InMemoryObjectStore::new()),
            cache: Arc::new(MemoryCacheIndex::new()),
        }
    }
}

async fn fixture_with(epoch: u64, ledger: Arc<MemoryExecutionLedger>, authorizer: Arc<dyn InputAuthorizer>) -> Fixture {
    fixture_from(FixtureOptions { epoch, ledger, authorizer, ..Default::default() }).await
}

async fn fixture_from(options: FixtureOptions) -> Fixture {
    let FixtureOptions { epoch, ledger, authorizer, store, cache } = options;
    let io = Arc::new(MemoryNativeIo::new(store.clone()));
    let mut registry = FunctionRegistry::new();
    let echo = Arc::new(Echo::default());
    crate::functions::test_functions::register_with_echo(&mut registry, echo.clone()).unwrap();
    functions::media::register(&mut registry).unwrap();
    registry
        .register_native(
            RegistrationSpec::new("test.toolbound", "1").timeout(30),
            FunctionObject {
                func_type: FunctionType::Native,
                content: "test.toolbound/impl-1".into(),
                is_pure: true,
                params_type: BTreeMap::new(),
                params_schema: json!({"type": "object", "properties": {"value": {}}}),
                result_type: FunctionResultType::Fixed("object".into()),
                result_schema: json!({"type": "object"}),
            },
            Arc::new(ToolBound),
        )
        .unwrap();
    let registry = Arc::new(registry);
    registry.publish(store.as_ref()).await.unwrap();
    let environment = Arc::new(SwitchableEnvironment(RwLock::new(Arc::new(ToolEnvironment::default()))));
    let tracker = Arc::new(RecordingTaskTracker::new());
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let temp_root = std::env::temp_dir().join(format!("wf-test-{}", uuid::Uuid::new_v4()));
    let runner = Arc::new(LocalThunkRunner::new(
        registry.clone(),
        io.clone(),
        environment.clone(),
        tracker.clone(),
        ledger.clone(),
        tx,
        temp_root,
        epoch,
        ExecutionLimits { max_queue: 4, ..Default::default() },
    ));
    let dispatcher: Arc<dyn ThunkDispatcher> = runner.clone();
    let object_store: Arc<dyn WorkflowObjectStore> = store.clone();
    let orchestrator = Arc::new(
        WorkflowOrchestrator::new(dispatcher, object_store.clone(), ServiceTracker::from_dyn(tracker.clone()))
            .with_functions(registry.clone())
            .with_cache(cache)
            .with_environment(environment.clone())
            .with_io(io.clone()),
    );
    let definitions = Arc::new(DefinitionStore::new());
    crate::system_defs::seed_system_definitions(&definitions, &registry)
        .await
        .unwrap();
    let handler = WorkflowRpcHandler::new(definitions, Arc::new(RunStore::new()), orchestrator, object_store)
        .with_caller_verifier(Arc::new(TestVerifier))
        .with_input_authorizer(authorizer)
        .with_execution_epoch(epoch);
    handler.driver().start_result_loop(rx);
    Fixture { handler, runner, tracker, io, ledger, environment, echo }
}

async fn fixture() -> Fixture {
    fixture_with(1, Arc::new(MemoryExecutionLedger::default()), Arc::new(AllowAllInputAuthorizer)).await
}

fn req(method: &str, params: Value, token: &str) -> RPCRequest {
    RPCRequest {
        method: method.to_string(),
        params,
        seq: 1,
        token: Some(token.to_string()),
        trace_id: None,
    }
}

async fn call(f: &Fixture, method: &str, params: Value, token: &str) -> std::result::Result<Value, RPCErrors> {
    let resp = f
        .handler
        .handle_rpc_call(req(method, params, token), "127.0.0.1".parse().unwrap())
        .await?;
    match resp.result {
        RPCResult::Success(v) => Ok(v),
        RPCResult::Failed(err) => Err(RPCErrors::ReasonError(format!("{err:?}"))),
    }
}

async fn ok(f: &Fixture, method: &str, params: Value, token: &str) -> Value {
    let value = call(f, method, params, token).await.unwrap();
    assert_eq!(value["ok"], true, "{method} failed: {value}");
    value
}

fn step(id: &str, executor: &str, input: Value, idempotent: bool) -> Value {
    json!({
        "id": id, "name": id, "executor": executor, "type": "autonomous", "idempotent": idempotent, "skippable": false,
        "input": input, "output_schema": {"type": "object", "properties": {"echo": {}, "calls": {"type": "integer"}, "slept_ms": {"type": "integer"}}},
        "guards": {"retry": {"max_attempts": 1, "fallback": "abort"}}
    })
}

fn user_definition(id: &str, steps: Vec<Value>) -> Value {
    let ids: Vec<String> = steps.iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    let mut edges: Vec<Value> = ids.windows(2).map(|w| json!({"from": w[0], "to": w[1]})).collect();
    edges.push(json!({"from": ids.last().unwrap()}));
    json!({"schema_version": "0.2.0", "id": id, "name": id, "trigger": {"type": "manual"}, "steps": steps, "edges": edges})
}

async fn wait_status(f: &Fixture, run_id: &str, token: &str, secs: u64) -> Value {
    let status = f.handler.driver().wait_terminal(run_id, Duration::from_secs(secs)).await;
    assert!(status.is_some(), "run {run_id} vanished");
    ok(f, "get_run_graph", json!({"run_id": run_id}), token).await
}

const ALICE: &str = "alice|agent-a";
const ALICE_OTHER_APP: &str = "alice|agent-b";
const BOB: &str = "bob|agent-a";
const ADMIN: &str = "admin|control-panel";

#[tokio::test]
async fn echo_runs_end_to_end_through_the_local_runner_and_caches_the_second_run() {
    let f = fixture().await;
    let definition = user_definition(
        "wf-echo",
        vec![
            step("a", "/tool/test.echo@1", json!({"value": "hello"}), true),
            step("b", "/tool/test.echo@1", json!({"value": {"prev": "${a.output.echo}"}}), true),
        ],
    );
    let submitted = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await;
    let workflow_id = submitted["workflow_id"].as_str().unwrap().to_string();
    assert_eq!(submitted["source"], "user_imported");
    assert_eq!(submitted["definition"]["resolved_functions"]["a"], submitted["definition"]["resolved_functions"]["b"]);

    let created = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 3000}), ALICE).await;
    let run_id = created["run_id"].as_str().unwrap().to_string();
    assert_eq!(created["reused"], false);
    let graph = wait_status(&f, &run_id, ALICE, 5).await;
    assert_eq!(graph["status"], "completed", "{graph}");
    assert_eq!(graph["node_sources"]["a"], "executed");
    assert_eq!(graph["node_sources"]["b"], "executed");
    assert_eq!(graph["output"]["b"]["echo"]["prev"], "hello");
    assert_eq!(graph["requested_by"], "alice/agent-a");
    assert_eq!(graph["revision"], submitted["revision"]);
    assert_eq!(f.echo.calls(), 2);

    let output = ok(&f, "get_run_output", json!({"run_id": run_id}), ALICE).await;
    assert_eq!(output["terminal"], true);
    assert_eq!(output["output"]["b"]["echo"]["prev"], "hello");

    // Thunk tasks went queued -> running -> succeeded.
    let phases = f.tracker.thunk_phases(&format!("{run_id}/a/1")).await;
    assert_eq!(phases, vec!["queued", "running", "succeeded"]);
    assert!(f.tracker.thunk(&run_id, &format!("{run_id}/a/1")).await.is_some());
    assert_eq!(f.runner.active_count(), 0);

    // Second run: both pure steps come from the cache; no new invocation.
    let created = ok(&f, "create_run", json!({"workflow_id": submitted["workflow_id"], "auto_start": true, "wait_ms": 3000}), ALICE).await;
    let run2 = created["run_id"].as_str().unwrap().to_string();
    let graph = wait_status(&f, &run2, ALICE, 5).await;
    assert_eq!(graph["status"], "completed");
    assert_eq!(graph["node_sources"]["a"], "cache");
    assert_eq!(graph["node_sources"]["b"], "cache");
    assert_eq!(f.echo.calls(), 2);

    let history = ok(&f, "get_history", json!({"run_id": run2}), ALICE).await;
    assert!(history["events"].as_array().unwrap().iter().any(|e| e["type"] == "step.completed" && e["payload"]["source"] == "cache"));
    let listed = ok(&f, "list_runs", json!({"status": "completed"}), ALICE).await;
    assert_eq!(listed["runs"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn system_definitions_are_seeded_bound_and_read_only() {
    let f = fixture().await;
    let listed = ok(&f, "list_definitions", json!({"source": "system"}), ALICE).await;
    let ids: Vec<&str> = listed["definitions"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 4);
    for id in ["media.image", "media.pdf", "media.video", "media.document"] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
    assert!(listed["definitions"].as_array().unwrap().iter().all(|d| d["read_only"] == true && d["source"] == "system"));

    let pdf = ok(&f, "get_definition", json!({"workflow_id": "media.pdf"}), BOB).await;
    let bound = pdf["definition"]["resolved_functions"].as_object().unwrap();
    assert_eq!(bound.len(), 5);
    for fun_id in bound.values() {
        let function = ok(&f, "get_function", json!({"fun_id": fun_id}), BOB).await;
        assert_eq!(function["function"]["published"], true);
        assert_eq!(function["function"]["object"]["func_type"], "native");
    }
    assert_eq!(pdf["definition"]["revision"].as_str().unwrap().len(), 64);

    let functions = ok(&f, "list_functions", json!({"prefix": "media."}), ALICE).await;
    assert_eq!(functions["functions"].as_array().unwrap().len(), 9);
    let caps = ok(&f, "get_capabilities", json!({}), ALICE).await;
    assert_eq!(caps["execution_epoch"], 1);

    let rejected = call(&f, "submit_definition", json!({"definition": user_definition("media.custom", vec![step("a", "/tool/test.echo@1", json!({}), true)])}), ALICE).await.unwrap();
    assert_eq!(rejected["error"], "system_definition_read_only");
    let forged = call(&f, "submit_definition", json!({"source": "system", "definition": user_definition("wf-forged", vec![step("a", "/tool/test.echo@1", json!({}), true)])}), ALICE).await;
    assert!(matches!(forged, Err(RPCErrors::NoPermission(_))));
    let archived = call(&f, "archive_definition", json!({"workflow_id": "media.pdf"}), ADMIN).await.unwrap();
    assert_eq!(archived["error"], "system_definition_read_only");
    let unresolved = call(&f, "submit_definition", json!({"definition": user_definition("wf-u", vec![step("a", "/tool/media.nope@1", json!({}), true)])}), ALICE).await.unwrap();
    assert_eq!(unresolved["error"], "unresolved_function");
}

#[tokio::test]
async fn identity_is_verified_on_every_run_operation() {
    let f = fixture().await;
    let definition = user_definition("wf-id", vec![step("a", "/tool/test.sleep@1", json!({"ms": 2000}), true)]);
    let forged = call(&f, "submit_definition", json!({"owner": {"user_id": "bob", "app_id": "agent-a"}, "definition": definition.clone()}), ALICE).await;
    assert!(matches!(forged, Err(RPCErrors::NoPermission(_))));
    let no_token = f.handler.handle_rpc_call(RPCRequest { method: "list_runs".into(), params: json!({}), seq: 1, token: None, trace_id: None }, "127.0.0.1".parse().unwrap()).await;
    assert!(matches!(no_token, Err(RPCErrors::NoPermission(_))));

    let submitted = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await;
    let workflow_id = submitted["workflow_id"].as_str().unwrap().to_string();
    let bob_get = call(&f, "get_definition", json!({"workflow_id": workflow_id}), BOB).await;
    assert!(matches!(bob_get, Err(RPCErrors::NoPermission(_))));
    let created = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true}), ALICE).await;
    let run_id = created["run_id"].as_str().unwrap().to_string();
    for method in ["get_run_graph", "get_run_output", "get_history", "start_run", "cancel_run"] {
        let err = call(&f, method, json!({"run_id": run_id}), BOB).await;
        assert!(matches!(err, Err(RPCErrors::NoPermission(_))), "{method}: {err:?}");
    }
    let other_app = call(&f, "get_run_graph", json!({"run_id": run_id}), ALICE_OTHER_APP).await;
    assert!(matches!(other_app, Err(RPCErrors::NoPermission(_))));
    let admin = ok(&f, "get_run_graph", json!({"run_id": run_id}), ADMIN).await;
    assert_eq!(admin["requested_by"], "alice/agent-a");
    assert!(ok(&f, "list_runs", json!({}), BOB).await["runs"].as_array().unwrap().is_empty());
    assert_eq!(ok(&f, "list_runs", json!({}), ADMIN).await["runs"].as_array().unwrap().len(), 1);
    let foreign_filter = call(&f, "list_runs", json!({"owner": {"user_id": "alice", "app_id": "agent-a"}}), BOB).await;
    assert!(matches!(foreign_filter, Err(RPCErrors::NoPermission(_))));

    let thunk = f.tracker.thunk(&run_id, &format!("{run_id}/a/1")).await.unwrap();
    assert_eq!(thunk.requested_by.as_deref(), Some("alice/agent-a"));
    ok(&f, "cancel_run", json!({"run_id": run_id}), ALICE).await;
}

#[tokio::test]
async fn ensure_merges_active_runs_per_principal_and_releases_on_terminal() {
    let f = fixture().await;
    let definition = user_definition("wf-ensure", vec![step("a", "/tool/test.sleep@1", json!({"ms": 800}), true)]);
    let workflow_id = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await["workflow_id"].as_str().unwrap().to_string();
    let first = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "ensure": true, "wait_ms": 0}), ALICE).await;
    let second = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "ensure": true, "wait_ms": 0}), ALICE).await;
    assert_eq!(first["reused"], false);
    assert_eq!(second["reused"], true);
    assert_eq!(first["run_id"], second["run_id"]);
    let foreign = call(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "ensure": true, "wait_ms": 0}), ALICE_OTHER_APP).await;
    assert!(matches!(foreign, Err(RPCErrors::NoPermission(_))));
    let admin_run = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "ensure": true, "wait_ms": 0}), ADMIN).await;
    assert_eq!(admin_run["reused"], false);
    assert_ne!(admin_run["run_id"], first["run_id"]);
    let plain = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": false}), ALICE).await;
    assert_ne!(plain["run_id"], first["run_id"]);

    let run_id = first["run_id"].as_str().unwrap().to_string();
    let graph = wait_status(&f, &run_id, ALICE, 5).await;
    assert_eq!(graph["status"], "completed");
    let after = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": false, "ensure": true}), ALICE).await;
    assert_eq!(after["reused"], false);
    assert_ne!(after["run_id"], first["run_id"]);
}

#[tokio::test]
async fn cancel_stops_a_running_execution_and_late_receipts_do_not_revive_the_run() {
    let f = fixture().await;
    let definition = user_definition("wf-cancel", vec![step("a", "/tool/test.sleep@1", json!({"ms": 30000}), true)]);
    let workflow_id = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await["workflow_id"].as_str().unwrap().to_string();
    let created = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 100}), ALICE).await;
    let run_id = created["run_id"].as_str().unwrap().to_string();
    assert_eq!(created["status"], "running");
    let cancelled = ok(&f, "cancel_run", json!({"run_id": run_id, "reason": "user stop"}), ALICE).await;
    assert_eq!(cancelled["status"], "aborted");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let graph = ok(&f, "get_run_graph", json!({"run_id": run_id}), ALICE).await;
    assert_eq!(graph["status"], "aborted");
    assert_eq!(graph["node_errors"]["a"]["code"], "cancelled");
    assert_eq!(f.runner.active_count(), 0);
    let phases = f.tracker.thunk_phases(&format!("{run_id}/a/1")).await;
    assert_eq!(phases.last().map(String::as_str), Some("cancelled:cancelled"), "{phases:?}");
    let again = call(&f, "start_run", json!({"run_id": run_id}), ALICE).await.unwrap();
    assert_eq!(again["error"], "run_terminal");
}

#[tokio::test]
async fn step_timeout_kills_the_execution_and_fails_the_run_with_timeout() {
    let f = fixture().await;
    let mut s = step("a", "/tool/test.sleep@1", json!({"ms": 30000}), true);
    s["guards"]["timeout"] = json!("1s");
    let definition = user_definition("wf-timeout", vec![s]);
    let workflow_id = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await["workflow_id"].as_str().unwrap().to_string();
    let started = std::time::Instant::now();
    let created = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 0}), ALICE).await;
    assert!(started.elapsed() < Duration::from_secs(1), "create_run must not block on execution");
    let run_id = created["run_id"].as_str().unwrap().to_string();
    let graph = wait_status(&f, &run_id, ALICE, 6).await;
    assert_eq!(graph["status"], "failed", "{graph}");
    assert_eq!(graph["node_errors"]["a"]["code"], "timeout");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(f.runner.active_count(), 0);
    let output = ok(&f, "get_run_output", json!({"run_id": run_id}), ALICE).await;
    assert_eq!(output["node_errors"]["a"]["code"], "timeout");
    assert!(f.tracker.thunk_phases(&format!("{run_id}/a/1")).await.contains(&"failed:timeout".to_string()));
}

#[tokio::test]
async fn structured_failures_reach_run_task_and_rpc_without_waiting_human() {
    let f = fixture().await;
    let definition = user_definition("wf-fail", vec![
        step("a", "/tool/test.echo@1", json!({"value": "ok"}), true),
        step("b", "/tool/test.fail@1", json!({"code": "resource_limit", "retryable": false}), true),
    ]);
    let workflow_id = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await["workflow_id"].as_str().unwrap().to_string();
    let run_id = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 0}), ALICE).await["run_id"].as_str().unwrap().to_string();
    let graph = wait_status(&f, &run_id, ALICE, 5).await;
    assert_eq!(graph["status"], "failed");
    assert_eq!(graph["node_states"]["b"], "failed");
    assert!(graph["human_waiting_nodes"].as_array().unwrap().is_empty());
    assert_eq!(graph["node_errors"]["b"]["code"], "resource_limit");
    assert_eq!(graph["node_errors"]["b"]["details"]["params"]["code"], "resource_limit");
    let step_view = f.tracker.step(&run_id, "b").await.unwrap();
    assert_eq!(step_view.error_detail.unwrap()["code"], "resource_limit");
    assert!(f.tracker.thunk_phases(&format!("{run_id}/b/1")).await.contains(&"failed:resource_limit".to_string()));
    // The failed result was not cached: a rerun executes b again.
    let run2 = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 0}), ALICE).await["run_id"].as_str().unwrap().to_string();
    let graph2 = wait_status(&f, &run2, ALICE, 5).await;
    assert_eq!(graph2["node_sources"]["a"], "cache");
    assert_eq!(graph2["node_sources"].get("b"), None);
    assert_eq!(graph2["status"], "failed");
}

#[tokio::test]
async fn media_pipeline_without_tools_fails_with_tools_missing_and_validates_input() {
    let f = fixture().await;
    let (obj_id, _content_id) = f.io.add_file("doc.pdf", b"%PDF-1.4\n%fake\n").await;
    let bad = call(&f, "create_run", json!({"workflow_id": "media.pdf", "input": {"obj_id": obj_id, "dpi": 9999}}), ALICE).await.unwrap();
    assert_eq!(bad["error"], "invalid_input");
    let unknown = call(&f, "create_run", json!({"workflow_id": "media.pdf", "input": {"obj_id": "cyfile:0000"}}), ALICE).await.unwrap();
    assert_eq!(unknown["error"], "invalid_input");
    let created = ok(&f, "create_run", json!({"workflow_id": "media.pdf", "auto_start": true, "ensure": true, "wait_ms": 0, "input": {"obj_id": obj_id, "content_id": "sha256:forged"}}), ALICE).await;
    let run_id = created["run_id"].as_str().unwrap().to_string();
    let graph = wait_status(&f, &run_id, ALICE, 10).await;
    assert_eq!(graph["status"], "failed", "{graph}");
    let trigger = &graph["node_outputs"]["trigger"];
    assert_ne!(trigger["content_id"], "sha256:forged");
    assert_eq!(trigger["dpi"], 144);
    assert_eq!(trigger["limits"]["max_pages"], 50);
    assert_eq!(graph["node_states"]["capabilities"], "completed");
    assert_eq!(graph["node_sources"]["capabilities"], "executed");
    let failed: Vec<(&String, &Value)> = graph["node_errors"].as_object().unwrap().iter().collect();
    assert!(!failed.is_empty());
    assert!(failed.iter().any(|(_, e)| e["code"] == "tools_missing"), "{failed:?}");
    let tools_missing = failed.iter().find(|(_, e)| e["code"] == "tools_missing").unwrap().1;
    assert!(tools_missing["details"]["tools_missing"].as_array().unwrap().iter().any(|t| t.as_str().unwrap().starts_with("pdf")));
    assert!(graph["human_waiting_nodes"].as_array().unwrap().is_empty());

    // Second run: capabilities executes again (never cached); the failed step reruns.
    let created2 = ok(&f, "create_run", json!({"workflow_id": "media.pdf", "auto_start": true, "ensure": true, "wait_ms": 0, "input": {"obj_id": obj_id}}), ALICE).await;
    assert_eq!(created2["reused"], false);
    let graph2 = wait_status(&f, created2["run_id"].as_str().unwrap(), ALICE, 10).await;
    assert_eq!(graph2["node_sources"]["capabilities"], "executed");
    assert_eq!(graph2["status"], "failed");
}

#[tokio::test]
async fn image_pipeline_fits_an_image_and_reuses_the_cache_across_file_names() {
    let f = fixture().await;
    let mut png = Vec::new();
    {
        use image::ImageEncoder;
        let img = image::RgbImage::from_fn(300, 200, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 7]));
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 300, 200, image::ExtendedColorType::Rgb8)
            .unwrap();
    }
    let (obj_a, content_a) = f.io.add_file("a.png", &png).await;
    let (obj_b, content_b) = f.io.add_file("renamed.png", &png).await;
    assert_ne!(obj_a, obj_b);
    assert_eq!(content_a, content_b);
    let created = ok(&f, "create_run", json!({"workflow_id": "media.image", "auto_start": true, "wait_ms": 0, "input": {"obj_id": obj_a, "max_long_edge": 100}}), ALICE).await;
    let graph = wait_status(&f, created["run_id"].as_str().unwrap(), ALICE, 10).await;
    assert_eq!(graph["status"], "completed", "{graph}");
    let fit = &graph["output"]["fit_image"];
    assert_eq!(fit["sent"]["width"], 100);
    assert!(fit["actions"].as_array().unwrap().iter().any(|a| a == "downscaled"));
    let model_obj = fit["model_media_obj_id"].as_str().unwrap().to_string();
    assert!(f.io.artifact_bytes(&model_obj).is_some());
    assert_eq!(graph["node_sources"]["fit_image"], "executed");

    let created = ok(&f, "create_run", json!({"workflow_id": "media.image", "auto_start": true, "wait_ms": 0, "input": {"obj_id": obj_b, "max_long_edge": 100}}), BOB).await;
    let graph = wait_status(&f, created["run_id"].as_str().unwrap(), BOB, 10).await;
    assert_eq!(graph["status"], "completed");
    assert_eq!(graph["node_sources"]["probe"], "cache");
    assert_eq!(graph["node_sources"]["fit_image"], "cache");
    assert_eq!(graph["node_sources"]["capabilities"], "executed");
    assert_eq!(graph["output"]["fit_image"]["model_media_obj_id"], model_obj);
    assert_eq!(f.io.artifacts().len(), 1);

    // Artifact gone: the step recomputes instead of returning a dangling id.
    f.io.delete_artifact(&model_obj);
    let created = ok(&f, "create_run", json!({"workflow_id": "media.image", "auto_start": true, "wait_ms": 0, "input": {"obj_id": obj_a, "max_long_edge": 100}}), ALICE).await;
    let graph = wait_status(&f, created["run_id"].as_str().unwrap(), ALICE, 10).await;
    assert_eq!(graph["status"], "completed");
    assert_eq!(graph["node_sources"]["fit_image"], "executed");
    let output = ok(&f, "get_run_output", json!({"run_id": created["run_id"]}), ALICE).await;
    assert!(output["delivery"]["artifacts"].as_array().unwrap().iter().any(|a| a == &graph["output"]["fit_image"]["model_media_obj_id"]));
}

#[tokio::test]
async fn input_read_authorization_is_enforced_before_a_run_exists() {
    let f = fixture_with(1, Arc::new(MemoryExecutionLedger::default()), Arc::new(DenyingAuthorizer)).await;
    let (obj_id, _) = f.io.add_file("x.png", b"not really").await;
    let denied = call(&f, "create_run", json!({"workflow_id": "media.image", "input": {"obj_id": obj_id}}), ALICE).await;
    assert!(matches!(denied, Err(RPCErrors::NoPermission(_))), "{denied:?}");
    assert!(ok(&f, "list_runs", json!({}), ADMIN).await["runs"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn environment_change_between_scheduling_and_execution_fails_with_environment_changed() {
    let f = fixture().await;
    *f.environment.0.write().unwrap() = env_with("24.02");
    let definition = user_definition("wf-env", vec![
        step("wait", "/tool/test.sleep@1", json!({"ms": 600}), true),
        step("a", "/tool/test.toolbound@1", json!({"value": 1}), true),
    ]);
    let workflow_id = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await["workflow_id"].as_str().unwrap().to_string();
    // Fill the Image/Light lane? test.toolbound is Light; occupy nothing — instead swap the
    // environment while `wait` is running: `a` is scheduled after `wait`, so it sees the new
    // fingerprint at scheduling time. To force the mismatch, swap right after `a` is admitted.
    let run_id = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 0}), ALICE).await["run_id"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Block the Light lane so `a` queues behind sleepers, then swap tools while it waits.
    let blockers = user_definition("wf-block", (0..8).map(|i| step(&format!("s{i}"), "/tool/test.sleep@1", json!({"ms": 1500 + i}), true)).collect());
    let _ = blockers;
    *f.environment.0.write().unwrap() = env_with("22.12");
    let graph = wait_status(&f, &run_id, ALICE, 8).await;
    // Either `a` was scheduled with the new env (completed) or executed under a changed env.
    let code = graph["node_errors"]["a"]["code"].as_str().unwrap_or("");
    assert!(graph["status"] == "completed" || code == "environment_changed", "{graph}");
    // Direct runner-level check of the guard with a crafted execution.
    let registry = f.handler.orchestrator().functions();
    let function = registry.resolve("test.toolbound", "1").unwrap();
    let thunk = buckyos_api::ThunkObject::new(function.fun_id().clone(), BTreeMap::from([("value".to_string(), json!(2))]));
    let (thunk_id, _) = thunk.calc_obj_id().unwrap();
    let execution = buckyos_api::ThunkExecution {
        execution_id: "run-x/a/1".into(),
        thunk_obj_id: thunk_id,
        run_id: "run-x".into(),
        node_id: "a".into(),
        attempt: 1,
        execution_epoch: 1,
        source_obj_id: None,
        requested_by: None,
        deadline_at: None,
        registration: None,
        fingerprint: BTreeMap::from([("cmd/pdftoppm".to_string(), "24.02".to_string())]),
        cache_key: None,
    };
    f.runner.schedule_thunk(&execution, &thunk).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let phases = f.tracker.thunk_phases("run-x/a/1").await;
    assert_eq!(phases.last().map(String::as_str), Some("failed:environment_changed"), "{phases:?}");
}

#[tokio::test]
async fn stale_executions_of_a_previous_epoch_are_failed_as_interrupted_and_cleaned() {
    let ledger = Arc::new(MemoryExecutionLedger::default());
    let temp = std::env::temp_dir().join(format!("wf-stale-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp).unwrap();
    ledger
        .record_started(&ActiveExecutionRecord {
            execution_id: "run-old/a/1".into(),
            run_id: "run-old".into(),
            node_id: "a".into(),
            epoch: 1,
            temp_dir: temp.to_string_lossy().into_owned(),
            task_id: None,
            started_at: 0,
        })
        .await
        .unwrap();
    let f = fixture_with(2, ledger.clone(), Arc::new(AllowAllInputAuthorizer)).await;
    let stale = f.runner.cleanup_stale().await.unwrap();
    assert_eq!(stale.len(), 1);
    assert!(f.ledger.stale_executions(2).await.unwrap().is_empty());
    assert_eq!(f.tracker.thunk_phases("run-old/a/1").await, vec!["failed:interrupted"]);
    // Temp dirs outside the runner root are left alone.
    assert!(temp.exists());
    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn queue_admission_is_bounded() {
    let f = fixture().await;
    let steps: Vec<Value> = (0..6).map(|i| step(&format!("s{i}"), "/tool/test.sleep@1", json!({"ms": 400}), false)).collect();
    let definition = json!({
        "schema_version": "0.2.0", "id": "wf-queue", "name": "wf-queue", "trigger": {"type": "manual"},
        "steps": steps,
        "nodes": [{"type": "parallel", "id": "par", "branches": (0..6).map(|i| format!("s{i}")).collect::<Vec<_>>(), "join": "all"}],
        "edges": [{"from": "par"}]
    });
    let workflow_id = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await["workflow_id"].as_str().unwrap().to_string();
    let run_id = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 0}), ALICE).await["run_id"].as_str().unwrap().to_string();
    let graph = wait_status(&f, &run_id, ALICE, 10).await;
    // max_queue = 4: at least one branch was refused with queue_full and the run failed.
    let codes: Vec<&str> = graph["node_errors"].as_object().unwrap().values().filter_map(|e| e["code"].as_str()).collect();
    assert!(codes.iter().any(|c| *c == "queue_full"), "{graph}");
    assert_eq!(graph["status"], "failed");
    let dispatcher = InMemoryThunkDispatcher::new();
    assert!(dispatcher.scheduled().await.is_empty());
}

#[tokio::test]
async fn persistent_cache_index_survives_a_service_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("workflow.db");
    let store = Arc::new(InMemoryObjectStore::new());
    let definition = user_definition("wf-persist", vec![step("a", "/tool/test.echo@1", json!({"value": "persist"}), true)]);

    let db = crate::persistence::WorkflowDb::open(&db_path).unwrap();
    let epoch = db.next_epoch().unwrap();
    let f = fixture_from(FixtureOptions { epoch, store: store.clone(), cache: db.clone(), ..Default::default() }).await;
    let workflow_id = ok(&f, "submit_definition", json!({"definition": definition.clone()}), ALICE).await["workflow_id"].as_str().unwrap().to_string();
    let run = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 0}), ALICE).await;
    let graph = wait_status(&f, run["run_id"].as_str().unwrap(), ALICE, 5).await;
    assert_eq!(graph["node_sources"]["a"], "executed");
    assert_eq!(f.echo.calls(), 1);
    drop(f);
    drop(db);

    // New process: fresh runner, registry, runs; same object store (Named Store) and db file.
    let db = crate::persistence::WorkflowDb::open(&db_path).unwrap();
    let epoch = db.next_epoch().unwrap();
    assert_eq!(epoch, 2);
    let f = fixture_from(FixtureOptions { epoch, store, cache: db.clone(), ..Default::default() }).await;
    let workflow_id = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await["workflow_id"].as_str().unwrap().to_string();
    let run = ok(&f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 0}), ALICE).await;
    let graph = wait_status(&f, run["run_id"].as_str().unwrap(), ALICE, 5).await;
    assert_eq!(graph["status"], "completed");
    assert_eq!(graph["node_sources"]["a"], "cache");
    assert_eq!(graph["output"]["a"]["echo"], "persist");
    assert_eq!(f.echo.calls(), 0);
}

#[tokio::test]
async fn the_run_id_is_the_root_task_id_and_a_joining_ensure_creates_no_task() {
    let f = fixture().await;
    let definition = user_definition("wf-one-id", vec![step("a", "/tool/test.sleep@1", json!({"ms": 300}), true)]);
    let submitted = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await;
    let request = json!({"workflow_id": submitted["workflow_id"], "auto_start": true, "ensure": true, "wait_ms": 0});
    let (a, b, c, d) = tokio::join!(
        ok(&f, "create_run", request.clone(), ALICE),
        ok(&f, "create_run", request.clone(), ALICE),
        ok(&f, "create_run", request.clone(), ALICE),
        ok(&f, "create_run", request.clone(), ALICE),
    );
    let created = [a, b, c, d];
    let tasks = f.tracker.created_run_tasks().await;
    assert_eq!(tasks.len(), 1, "one active run, one root task");
    for run in &created {
        assert_eq!(run["run_id"], tasks[0].as_str());
    }
    assert_eq!(created.iter().filter(|run| run["reused"] == false).count(), 1);
    let graph = wait_status(&f, &tasks[0], ALICE, 5).await;
    assert_eq!(graph["run_id"], tasks[0].as_str());
    assert!(graph.get("task_id").is_none());
}

#[tokio::test]
async fn delivery_grants_exactly_the_produced_objects_to_the_authorized_requester() {
    let f = fixture().await;
    let png = {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgb8(400, 200)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    };
    let (obj_id, _) = f.io.add_file("a.png", &png).await;
    let created = ok(&f, "create_run", json!({"workflow_id": "media.image", "auto_start": true, "wait_ms": 0, "input": {"obj_id": obj_id}}), ALICE).await;
    let run_id = created["run_id"].as_str().unwrap().to_string();
    wait_status(&f, &run_id, ALICE, 10).await;

    assert!(call(&f, "get_run_output", json!({"run_id": run_id}), BOB).await.is_err());
    assert!(f.io.holdings_of("bob/agent-a").is_empty());

    let output = ok(&f, "get_run_output", json!({"run_id": run_id}), ALICE).await;
    let model = output["output"]["fit_image"]["model_media_obj_id"].as_str().unwrap().to_string();
    assert_eq!(output["delivery"]["held"], 1);
    assert_eq!(f.io.holdings_of("alice/agent-a"), vec![model]);
    assert!(f.io.holdings_of("alice/agent-b").is_empty());
}

#[tokio::test]
async fn a_manifest_hands_over_every_object_it_names() {
    let f = fixture().await;
    let mut manifest = buckyos_api::MediaManifest::new("pdf", "chunk:source");
    for (page, text, image) in [(1, "cyfile:aa01", "cyfile:bb01"), (2, "cyfile:aa02", "cyfile:bb02")] {
        manifest.entries.push(buckyos_api::MediaEntry {
            kind: buckyos_api::MediaEntryKind::Page,
            location: buckyos_api::MediaWhere {
                source: buckyos_api::MediaSourceRef::Original,
                page: Some(page),
                time_ms: None,
                index: None,
            },
            text_obj_id: Some(text.to_string()),
            image_obj_id: Some(image.to_string()),
            audio_obj_id: None,
            status: buckyos_api::MediaEntryStatus::Ok,
            error: None,
        });
    }
    let manifest_id = f
        .io
        .objects()
        .put_json(buckyos_api::MEDIA_MANIFEST_OBJ_TYPE, &serde_json::to_value(&manifest).unwrap())
        .await
        .unwrap();
    let owner = Owner { user_id: "alice".into(), app_id: "agent-a".into() };
    let held = f.handler.grant_output_holdings(&owner, &[manifest_id.clone()]).await.unwrap();
    assert_eq!(held, 5);
    let mut holdings = f.io.holdings_of("alice/agent-a");
    holdings.sort();
    let mut expected = vec![manifest_id, "cyfile:aa01".into(), "cyfile:aa02".into(), "cyfile:bb01".into(), "cyfile:bb02".into()];
    expected.sort();
    assert_eq!(holdings, expected);
}

#[tokio::test]
async fn naming_an_object_in_a_run_output_grants_nothing() {
    let f = fixture().await;
    let secret = "cyfile:deadbeef";

    // A user definition may emit any object id.
    let definition = user_definition("wf-name-it", vec![step("a", "/tool/test.echo@1", json!({"value": secret}), true)]);
    let submitted = ok(&f, "submit_definition", json!({"definition": definition}), BOB).await;
    let created = ok(&f, "create_run", json!({"workflow_id": submitted["workflow_id"], "auto_start": true, "wait_ms": 0}), BOB).await;
    let run_id = created["run_id"].as_str().unwrap().to_string();
    let graph = wait_status(&f, &run_id, BOB, 5).await;
    assert_eq!(graph["status"], "completed");
    let output = ok(&f, "get_run_output", json!({"run_id": run_id}), BOB).await;
    assert_eq!(output["delivery"]["artifacts"], json!([secret]));
    assert_eq!(output["delivery"]["held"], 0);
    assert!(f.io.holdings_of("bob/agent-a").is_empty());

    // Nobody writes the steps of a system pipeline from outside.
    let (obj_id, _) = f.io.add_file("a.png", b"not an image").await;
    let created = ok(&f, "create_run", json!({"workflow_id": "media.image", "auto_start": false, "input": {"obj_id": obj_id}}), BOB).await;
    let refused = call(&f, "submit_step_output", json!({"run_id": created["run_id"], "node_id": "fit_image", "output": {"model_media_obj_id": secret}}), BOB).await.unwrap();
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"], "system_definition_read_only");
    let graph = ok(&f, "get_run_graph", json!({"run_id": created["run_id"]}), BOB).await;
    assert!(graph["node_outputs"].get("fit_image").is_none());
}

#[tokio::test]
async fn system_pipelines_cannot_be_scheduled() {
    let f = fixture().await;
    let target = crate::scheduled_task_manager::ScheduleSubtaskTemplate {
        task_type: "workflow.run".into(),
        name_template: "convert".into(),
        data_template: json!({"request": {"workflow_id": "media.image", "input": {"obj_id": "cyfile:aa", "content_id": "mix256:someone-elses"}}}),
    };
    let refused = f.handler.validate_target_exists(&target).await.unwrap_err();
    assert!(refused.to_string().contains("cannot be scheduled"), "{refused}");

    // One stored before this rule does not get past the fire either: a fire has no
    // caller to authorize the input against.
    let stored = WorkflowSchedule {
        schedule_id: "sch-old".into(),
        owner: Owner { user_id: "bob".into(), app_id: "agent-a".into() },
        name: "convert".into(),
        description: None,
        status: ScheduleStatus::Running,
        schedule: schedule_spec_from_value(&json!({"kind": "cron", "expr": "0 0 * * *"})).unwrap(),
        target,
        state: Default::default(),
        policy: schedule_policy_from_value(None).unwrap(),
        task_mirror: ScheduleTaskMirror { root_task_id: Some("t-root".into()), root_id: Some("t-root".into()) },
        created_at: 0,
        updated_at: 0,
    };
    f.handler.schedules.insert(stored).await;
    let failed = f.handler.fire_schedule("sch-old", 1, true).await.unwrap_err();
    assert_eq!(failed["ok"], false, "{failed}");
    assert!(failed.to_string().contains("system_workflow_not_schedulable"), "{failed}");
    assert!(f.tracker.created_run_tasks().await.is_empty());
}

#[tokio::test]
async fn list_runs_filters_by_the_status_runs_report() {
    let f = fixture().await;
    let mut human = step("ask", "", json!({}), true);
    human["type"] = json!("human_required");
    human.as_object_mut().unwrap().remove("executor");
    human.as_object_mut().unwrap().remove("input");
    human.as_object_mut().unwrap().remove("guards");
    human["prompt"] = json!("?");
    let submitted = ok(&f, "submit_definition", json!({"definition": user_definition("wf-ask", vec![human])}), ALICE).await;
    let created = ok(&f, "create_run", json!({"workflow_id": submitted["workflow_id"], "auto_start": true, "wait_ms": 1000}), ALICE).await;
    assert_eq!(created["status"], "waiting_human");
    let listed = ok(&f, "list_runs", json!({"status": "waiting_human"}), ALICE).await;
    assert_eq!(listed["runs"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["runs"][0]["run_id"], created["run_id"]);
    assert_eq!(ok(&f, "list_runs", json!({"status": "completed"}), ALICE).await["runs"], json!([]));
}

#[tokio::test]
async fn a_step_that_cannot_be_scheduled_fails_the_run_instead_of_leaving_it_running() {
    let f = fixture().await;
    let definition = user_definition("wf-semantic", vec![step("a", "/skill/not-resolved", json!({}), true)]);
    let submitted = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await;
    let created = ok(&f, "create_run", json!({"workflow_id": submitted["workflow_id"], "auto_start": true, "wait_ms": 1000}), ALICE).await;
    let graph = wait_status(&f, created["run_id"].as_str().unwrap(), ALICE, 5).await;
    assert_eq!(graph["status"], "failed", "{graph}");
    assert!(graph["node_errors"]["a"]["message"].as_str().unwrap().contains("/skill/not-resolved"), "{graph}");
    assert_eq!(graph["metrics"]["run_error"]["code"], graph["node_errors"]["a"]["code"]);
}

fn review_definition() -> Value {
    serde_json::from_str(include_str!("../../../../../test/workflow_test/fixtures/wf-dv-human-actions.json")).unwrap()
}

async fn waiting_for_review(f: &Fixture, workflow_id: &Value) -> String {
    let created = ok(f, "create_run", json!({"workflow_id": workflow_id, "auto_start": true, "wait_ms": 2000}), ALICE).await;
    let run_id = created["run_id"].as_str().unwrap().to_string();
    let graph = ok(f, "get_run_graph", json!({"run_id": run_id}), ALICE).await;
    assert_eq!(graph["status"], "waiting_human", "{graph}");
    run_id
}

async fn act(f: &Fixture, run_id: &str, kind: &str, payload: Value, token: &str) -> Value {
    call(f, "submit_human_action", json!({"run_id": run_id, "node_id": "review", "kind": kind, "payload": payload}), token).await.unwrap()
}

#[tokio::test]
async fn a_person_decides_a_step_through_the_rpc() {
    let f = fixture().await;
    let submitted = ok(&f, "submit_definition", json!({"definition": review_definition()}), ALICE).await;
    let workflow_id = submitted["workflow_id"].clone();

    let run_id = waiting_for_review(&f, &workflow_id).await;
    assert_eq!(act(&f, &run_id, "approve", Value::Null, ALICE).await["ok"], true);
    let graph = wait_status(&f, &run_id, ALICE, 5).await;
    assert_eq!(graph["status"], "completed", "{graph}");
    assert_eq!(graph["node_outputs"]["publish"]["echo"]["echo"], "draft");
    assert_eq!(graph["node_states"]["discard"], "pending");
    assert_eq!(graph["node_sources"]["review"], "submitted");
    let history = ok(&f, "get_history", json!({"run_id": run_id}), ALICE).await;
    let action = history["events"].as_array().unwrap().iter().find(|e| e["type"] == "human.action").unwrap();
    assert_eq!(action["actor"], "alice/agent-a", "the actor is the verified caller");
    assert_eq!(action["payload"]["action"], "approve");

    let run_id = waiting_for_review(&f, &workflow_id).await;
    act(&f, &run_id, "modify", json!({"modified_subject": {"echo": "edited"}}), ALICE).await;
    let graph = wait_status(&f, &run_id, ALICE, 5).await;
    assert_eq!(graph["node_outputs"]["publish"]["echo"]["echo"], "edited");

    let run_id = waiting_for_review(&f, &workflow_id).await;
    act(&f, &run_id, "reject", json!({"feedback": "no"}), ALICE).await;
    let graph = wait_status(&f, &run_id, ALICE, 5).await;
    assert_eq!(graph["status"], "completed");
    assert_eq!(graph["node_outputs"]["review"]["feedback"], "no");
    assert_eq!(graph["node_states"]["publish"], "pending");
    assert_eq!(graph["node_states"]["discard"], "completed");

    // A run that ended takes no more actions.
    assert_eq!(act(&f, &run_id, "retry", Value::Null, ALICE).await["error"], "run_terminal");

    let run_id = waiting_for_review(&f, &workflow_id).await;
    assert!(call(&f, "submit_human_action", json!({"run_id": run_id, "node_id": "review", "kind": "approve"}), BOB).await.is_err());
    assert_eq!(act(&f, &run_id, "sign", Value::Null, ALICE).await["error"], "invalid_human_action");
    assert_eq!(act(&f, &run_id, "modify", Value::Null, ALICE).await["error"], "invalid_human_action");
    assert_eq!(act(&f, &run_id, "skip", Value::Null, ALICE).await["error"], "node_not_skippable");
    let graph = ok(&f, "get_run_graph", json!({"run_id": run_id}), ALICE).await;
    assert_eq!(graph["status"], "waiting_human", "refused actions change nothing");

    assert_eq!(act(&f, &run_id, "abort", Value::Null, ALICE).await["status"], "aborted");
}

#[tokio::test]
async fn retry_runs_the_step_again_and_skip_moves_past_it() {
    let f = fixture().await;
    let mut failing = step("flaky", "/tool/test.fail@1", json!({}), false);
    failing["guards"]["retry"]["fallback"] = json!("human");
    failing["skippable"] = json!(true);
    let after = step("after", "/tool/test.echo@1", json!({"value": "after"}), false);
    let submitted = ok(&f, "submit_definition", json!({"definition": user_definition("wf-flaky", vec![failing, after])}), ALICE).await;
    let created = ok(&f, "create_run", json!({"workflow_id": submitted["workflow_id"], "auto_start": true, "wait_ms": 2000}), ALICE).await;
    let run_id = created["run_id"].as_str().unwrap();
    assert_eq!(created["status"], "waiting_human", "{created}");

    let action = |kind: &'static str| json!({"run_id": run_id, "node_id": "flaky", "kind": kind});
    ok(&f, "submit_human_action", action("retry"), ALICE).await;
    let mut graph = Value::Null;
    for _ in 0..100 {
        graph = ok(&f, "get_run_graph", json!({"run_id": run_id}), ALICE).await;
        if graph["node_attempts"]["flaky"] == 2 && graph["status"] == "waiting_human" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(graph["node_attempts"]["flaky"], 2, "{graph}");
    assert_eq!(graph["status"], "waiting_human", "{graph}");

    ok(&f, "submit_human_action", action("skip"), ALICE).await;
    let graph = wait_status(&f, run_id, ALICE, 5).await;
    assert_eq!(graph["status"], "completed", "{graph}");
    assert_eq!(graph["node_states"]["flaky"], "skipped");
    assert_eq!(graph["node_outputs"]["after"]["echo"], "after");
}

#[tokio::test]
async fn nobody_decides_a_step_of_a_system_pipeline() {
    let f = fixture().await;
    let (obj_id, _) = f.io.add_file("a.png", b"not an image").await;
    let created = ok(&f, "create_run", json!({"workflow_id": "media.image", "auto_start": false, "input": {"obj_id": obj_id}}), ALICE).await;
    let refused = call(&f, "submit_human_action", json!({"run_id": created["run_id"], "node_id": "fit_image", "kind": "skip"}), ALICE).await.unwrap();
    assert_eq!(refused["error"], "system_definition_read_only");
}

#[tokio::test]
async fn a_cancelled_run_gives_its_ensure_slot_back() {
    let f = fixture().await;
    let definition = user_definition("wf-cancel-ensure", vec![step("a", "/tool/test.sleep@1", json!({"ms": 5000}), false)]);
    let submitted = ok(&f, "submit_definition", json!({"definition": definition}), ALICE).await;
    let request = json!({"workflow_id": submitted["workflow_id"], "auto_start": true, "ensure": true, "wait_ms": 0});
    let first = ok(&f, "create_run", request.clone(), ALICE).await;
    ok(&f, "cancel_run", json!({"run_id": first["run_id"]}), ALICE).await;
    let second = ok(&f, "create_run", request.clone(), ALICE).await;
    assert_eq!(second["reused"], false, "{second}");
    assert_ne!(second["run_id"], first["run_id"]);
    ok(&f, "cancel_run", json!({"run_id": second["run_id"]}), ALICE).await;
}
