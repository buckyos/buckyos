//! kRPC 入口分发与各 method 的真正实现。
//!
//! 方法清单与 [doc/workflow/workflow service.md](../../../../doc/workflow/workflow%20service.md) §3
//! 严格对齐：
//!
//! - §3.1 Definition：`submit_definition` / `get_definition` / `list_definitions` /
//!   `archive_definition` / `dry_run`
//! - §3.2 Run 生命周期：`create_run` / `start_run` / `tick_run` /
//!   `get_run_graph` / `list_runs`（pause/resume/cancel/状态读取退化为
//!   task_manager 写 TaskData，**不**在这里暴露）
//! - §3.4 Agent / 外部回调：`submit_step_output` / `report_step_progress` /
//!   `request_human`
//! - §3.4 Amendment：`submit_amendment` / `approve_amendment` /
//!   `reject_amendment`
//! - §3.5 事件：`get_history` / `subscribe_events`
//!
//! `service.<method>` 与裸 `<method>` 两种方法名都接受，前者由 `service::workflow`
//! 形态调用方使用，后者由直连 HTTP 客户端使用——同 msg_center / aicc 的惯例。

use ::kRPC::*;
use buckyos_api::{
    apply_schema_defaults, get_buckyos_api_runtime, validate_json_schema,
    validate_verify_hub_token_claims, TokenUse, UserType, WorkflowDefinition,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;

use crate::analysis::{trigger_input_schema, trigger_output_schema};
use crate::driver::RunDriver;
use crate::object_store::WorkflowObjectStore;
use crate::orchestrator::RunOptions;
use crate::{
    analyze_workflow, compile_workflow_with, AnalysisReport, CompiledWorkflow, ThunkDispatcher, WorkflowError, WorkflowOrchestrator,
};

use crate::scheduled_task_manager::{
    due_fire_times, is_reboot_schedule, next_fire_after, next_fire_times, render_subtask_template,
    rfc3339, schedule_policy_from_value, schedule_spec_from_value, schedule_target_from_value,
    schedule_workflow_id, validate_subtask_template, FireStatus, MisfirePolicy, ScheduleFireRecord,
    ScheduleSpec, ScheduleState, ScheduleStatus, ScheduleStore, ScheduleTarget, ScheduleTaskMirror,
    ScheduleTaskMirrorClient, WorkflowSchedule,
};
use crate::state::{
    is_reserved_definition_id, workflow_error_payload, AmendmentRecord, AmendmentStatus,
    DefinitionRecord, DefinitionSource, DefinitionStatus, DefinitionStore, EnsureOutcome, Owner,
    RunHandle, RunRecord, RunStore, ServiceTracker,
};
use crate::subscriptions::RunSubscriptionManager;

type RpcResult<T> = std::result::Result<T, RPCErrors>;

/// Verified caller of a workflow RPC: the principal every Definition / Run
/// operation is authorized against (TODO D13).
#[derive(Debug, Clone)]
pub struct WorkflowCaller {
    pub owner: Owner,
    pub is_admin: bool,
}

impl WorkflowCaller {
    pub fn principal(&self) -> String {
        self.owner.principal()
    }

    fn may_access(&self, owner: &Owner) -> bool {
        self.is_admin || self.owner == *owner
    }
}

#[async_trait::async_trait]
pub trait WorkflowCallerVerifier: Send + Sync {
    async fn verify(&self, ctx: &RPCContext) -> RpcResult<WorkflowCaller>;
}

/// Checks that a caller may read a run input object (TODO D3): knowing an
/// obj_id is not a read right.
#[async_trait::async_trait]
pub trait InputAuthorizer: Send + Sync {
    async fn authorize_read(
        &self,
        caller: &WorkflowCaller,
        token: Option<&str>,
        obj_id: &str,
    ) -> RpcResult<()>;
}

/// Tests / single-principal deployments: every verified caller may read.
pub struct AllowAllInputAuthorizer;

#[async_trait::async_trait]
impl InputAuthorizer for AllowAllInputAuthorizer {
    async fn authorize_read(&self, _: &WorkflowCaller, _: Option<&str>, _: &str) -> RpcResult<()> {
        Ok(())
    }
}

/// Asks the NDM proxy with the caller's own session token whether the
/// object is readable by that principal.
pub struct NdmInputAuthorizer;

#[async_trait::async_trait]
impl InputAuthorizer for NdmInputAuthorizer {
    async fn authorize_read(
        &self,
        caller: &WorkflowCaller,
        token: Option<&str>,
        obj_id: &str,
    ) -> RpcResult<()> {
        let token = token.ok_or_else(|| {
            RPCErrors::NoPermission("input authorization needs the caller session token".into())
        })?;
        let runtime = get_buckyos_api_runtime()?;
        let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| RPCErrors::InvalidToken("invalid session token".into()))?;
        authorization.set_sensitive(true);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| RPCErrors::ReasonError(format!("ndm client: {e}")))?;
        let ndm = ndn_toolkit::NdmClient::with_client(
            ndn_toolkit::NdmClientConfig {
                base_url: runtime.named_store_base_url(),
            },
            client,
        );
        let id = ndn_lib::ObjId::new(obj_id)
            .map_err(|e| RPCErrors::ParseRequestError(format!("invalid obj_id: {e}")))?;
        // `query_object_by_id` is a read-class NDM call: it succeeds only for a
        // holder of the object or a principal with `obj://ndm/objects` read.
        // Existence probes (`is_object_stored`) are open to any uploader and
        // must not be used here.
        match ndm.query_object_by_id(&id).await {
            Ok(named_store::ObjectState::NotExist) => Err(RPCErrors::NoPermission(format!(
                "{} cannot read `{obj_id}`",
                caller.principal()
            ))),
            Ok(_) => Ok(()),
            Err(err) => Err(RPCErrors::NoPermission(format!(
                "{} cannot read `{obj_id}`: {err}",
                caller.principal()
            ))),
        }
    }
}

struct RuntimeWorkflowCallerVerifier;

#[async_trait::async_trait]
impl WorkflowCallerVerifier for RuntimeWorkflowCallerVerifier {
    async fn verify(&self, ctx: &RPCContext) -> RpcResult<WorkflowCaller> {
        let token = ctx
            .token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                RPCErrors::NoPermission("workflow requires a session token".to_string())
            })?;
        let runtime = get_buckyos_api_runtime()?;
        let verified = runtime.verify_trusted_session_token(token).await?;
        let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session)?;
        let user_id = verified
            .sub
            .clone()
            .ok_or_else(|| RPCErrors::InvalidToken("workflow token has no subject".to_string()))?;
        let app_id = claims.target.canonical_key();
        if user_id.trim().is_empty() || app_id.trim().is_empty() {
            return Err(RPCErrors::InvalidToken(
                "workflow caller identity is incomplete".to_string(),
            ));
        }
        let mut is_admin = verified.sudo;
        if !is_admin {
            if let Ok(client) = runtime.get_control_panel_client().await {
                if let Ok(settings) = client.get_user_settings_by_username(&user_id).await {
                    is_admin = matches!(settings.user_type, UserType::Admin | UserType::Root);
                }
            }
        }
        Ok(WorkflowCaller {
            owner: Owner { user_id, app_id },
            is_admin,
        })
    }
}

pub type ServiceOrchestrator =
    WorkflowOrchestrator<dyn ThunkDispatcher, dyn WorkflowObjectStore, ServiceTracker>;

/// Default short wait before `create_run` returns a still-running run.
const CREATE_RUN_WAIT_MS: u64 = 200;
/// Lifetime of artifact read grants issued by `get_run_output`.
const DELIVERY_TTL_SECS: i64 = 6 * 3600;
/// In-memory retention of terminal runs (TODO D7): count and age caps.
const MAX_TERMINAL_RUNS: usize = 2000;
const TERMINAL_RUN_MAX_AGE_SECS: i64 = 7 * 86_400;
const RUN_PRUNE_EVERY_TICKS: u64 = 60;

/// 把 method dispatch + 各 method 的真正实现集中起来。
pub struct WorkflowRpcHandler {
    definitions: Arc<DefinitionStore>,
    runs: Arc<RunStore>,
    schedules: Arc<ScheduleStore>,
    orchestrator: Arc<ServiceOrchestrator>,
    driver: Arc<RunDriver>,
    object_store: Arc<dyn WorkflowObjectStore>,
    schedule_mirror: Option<Arc<ScheduleTaskMirrorClient>>,
    caller_verifier: Arc<dyn WorkflowCallerVerifier>,
    input_authorizer: Arc<dyn InputAuthorizer>,
    execution_epoch: u64,
    /// schedule 内存投影是否已从 Task DB 灌入。boot 时 task_mgr 可能尚未就绪，
    /// 加载会失败；由 scan 循环每 tick 重试直到成功（对齐 kevent+sweep 兜底）。
    schedules_hydrated: std::sync::atomic::AtomicBool,
    /// 仅用于把 hydrate 失败的 WARN 节流成首次一条，避免每秒刷屏。
    schedules_hydrate_warned: std::sync::atomic::AtomicBool,
    scan_ticks: std::sync::atomic::AtomicU64,
    subscriptions: Option<Arc<RunSubscriptionManager>>,
}

impl WorkflowRpcHandler {
    pub fn new(
        definitions: Arc<DefinitionStore>,
        runs: Arc<RunStore>,
        orchestrator: Arc<ServiceOrchestrator>,
        object_store: Arc<dyn WorkflowObjectStore>,
    ) -> Self {
        let driver = RunDriver::new(
            definitions.clone(),
            runs.clone(),
            orchestrator.clone(),
            None,
        );
        Self {
            definitions,
            runs,
            schedules: Arc::new(ScheduleStore::new_memory()),
            orchestrator,
            driver,
            object_store,
            schedule_mirror: None,
            caller_verifier: Arc::new(RuntimeWorkflowCallerVerifier),
            input_authorizer: Arc::new(AllowAllInputAuthorizer),
            execution_epoch: 0,
            schedules_hydrated: std::sync::atomic::AtomicBool::new(false),
            schedules_hydrate_warned: std::sync::atomic::AtomicBool::new(false),
            scan_ticks: std::sync::atomic::AtomicU64::new(0),
            subscriptions: None,
        }
    }

    pub fn with_schedules(mut self, schedules: Arc<ScheduleStore>) -> Self {
        self.schedules = schedules;
        self
    }

    pub fn with_schedule_mirror(mut self, mirror: Arc<ScheduleTaskMirrorClient>) -> Self {
        self.schedule_mirror = Some(mirror);
        self
    }

    #[cfg(test)]
    pub fn with_caller_verifier(mut self, verifier: Arc<dyn WorkflowCallerVerifier>) -> Self {
        self.caller_verifier = verifier;
        self
    }

    pub fn with_input_authorizer(mut self, authorizer: Arc<dyn InputAuthorizer>) -> Self {
        self.input_authorizer = authorizer;
        self
    }

    pub fn with_execution_epoch(mut self, epoch: u64) -> Self {
        self.execution_epoch = epoch;
        self
    }

    pub fn with_subscriptions(mut self, subscriptions: Arc<RunSubscriptionManager>) -> Self {
        self.driver = RunDriver::new(
            self.definitions.clone(),
            self.runs.clone(),
            self.orchestrator.clone(),
            Some(subscriptions.clone()),
        );
        self.subscriptions = Some(subscriptions);
        self
    }

    pub fn driver(&self) -> &Arc<RunDriver> {
        &self.driver
    }

    #[cfg(test)]
    pub fn orchestrator(&self) -> &Arc<ServiceOrchestrator> {
        &self.orchestrator
    }

    #[cfg(test)]
    pub fn definitions(&self) -> &Arc<DefinitionStore> {
        &self.definitions
    }

    pub async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> RpcResult<RPCResponse> {
        let method = canonical_method(&req.method);
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match method {
            // §3.1 Definition
            "submit_definition" => self.submit_definition(&req.params, &ctx).await,
            "get_definition" => self.get_definition(&req.params, &ctx).await,
            "list_definitions" => self.list_definitions(&req.params, &ctx).await,
            "archive_definition" => self.archive_definition(&req.params, &ctx).await,
            "dry_run" => self.dry_run(&req.params, &ctx).await,
            "list_functions" => self.list_functions(&req.params, &ctx).await,
            "get_function" => self.get_function(&req.params, &ctx).await,
            "get_capabilities" => self.get_capabilities(&ctx).await,
            // §3.2 Run lifecycle
            "create_run" => self.create_run(&req.params, &ctx).await,
            "start_run" => self.start_run(&req.params, &ctx).await,
            "tick_run" => self.tick_run(&req.params, &ctx).await,
            "cancel_run" => self.cancel_run(&req.params, &ctx).await,
            "get_run_graph" => self.get_run_graph(&req.params, &ctx).await,
            "get_run_output" => self.get_run_output(&req.params, &ctx).await,
            "list_runs" => self.list_runs(&req.params, &ctx).await,
            // §3.4 Agent
            "submit_step_output" => self.submit_step_output(&req.params, &ctx).await,
            "report_step_progress" => self.report_step_progress(&req.params, &ctx).await,
            "request_human" => self.request_human(&req.params, &ctx).await,
            // §3.4 Amendment
            "submit_amendment" => self.submit_amendment(&req.params, &ctx).await,
            "approve_amendment" => self.approve_amendment(&req.params, &ctx).await,
            "reject_amendment" => self.reject_amendment(&req.params, &ctx).await,
            // §3.5 Events
            "get_history" => self.get_history(&req.params, &ctx).await,
            "subscribe_events" => self.subscribe_events(&req.params, &ctx).await,
            // Schedule / Trigger
            "create_scheduled_task" => {
                let owner = self.authenticated_schedule_owner(&req.params, &ctx).await?;
                self.create_scheduled_task(&req.params, owner).await
            }
            "update_scheduled_task" => self.update_scheduled_task(&req.params, &ctx).await,
            "get_scheduled_task" => self.get_scheduled_task(&req.params, &ctx).await,
            "list_scheduled_tasks" => self.list_scheduled_tasks(&req.params, &ctx).await,
            "pause_scheduled_task" => self.pause_scheduled_task(&req.params, &ctx).await,
            "resume_scheduled_task" => self.resume_scheduled_task(&req.params, &ctx).await,
            "archive_scheduled_task" => self.archive_scheduled_task(&req.params, &ctx).await,
            "run_scheduled_task_now" => self.run_scheduled_task_now(&req.params, &ctx).await,
            "get_scheduled_task_history" => {
                self.get_scheduled_task_history(&req.params, &ctx).await
            }
            "validate_scheduled_task" => self.validate_scheduled_task(&req.params).await,
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        match result {
            Ok(value) => Ok(RPCResponse {
                result: RPCResult::Success(value),
                seq: req.seq,
                trace_id: req.trace_id,
            }),
            Err(err) => Err(err),
        }
    }

    // ----- identity helpers -----------------------------------------------

    async fn caller(&self, ctx: &RPCContext) -> RpcResult<WorkflowCaller> {
        self.caller_verifier.verify(ctx).await
    }

    /// Verified caller; `params.owner`, when present, must match it.
    async fn caller_matching_owner(
        &self,
        params: &Value,
        ctx: &RPCContext,
    ) -> RpcResult<WorkflowCaller> {
        let caller = self.caller(ctx).await?;
        if let Some(requested) = optional_owner(params) {
            if requested != caller.owner {
                return Err(RPCErrors::NoPermission(format!(
                    "owner {}:{} does not match authenticated caller {}:{}",
                    requested.user_id, requested.app_id, caller.owner.user_id, caller.owner.app_id
                )));
            }
        }
        Ok(caller)
    }

    fn require_definition_access(
        &self,
        caller: &WorkflowCaller,
        record: &DefinitionRecord,
    ) -> RpcResult<()> {
        if record.is_system() || caller.may_access(&record.owner) {
            return Ok(());
        }
        Err(RPCErrors::NoPermission(format!(
            "{} may not access workflow {}",
            caller.principal(),
            record.id
        )))
    }

    fn require_run_access(&self, caller: &WorkflowCaller, handle: &RunHandle) -> RpcResult<()> {
        if caller.may_access(&handle.owner) {
            return Ok(());
        }
        Err(RPCErrors::NoPermission(format!(
            "{} may not access this run",
            caller.principal()
        )))
    }

    // ----- §3.1 Workflow Definition --------------------------------------

    async fn submit_definition(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller_matching_owner(params, ctx).await?;
        let definition = require_definition(params)?;
        if is_reserved_definition_id(&definition.id) || is_reserved_definition_id(&definition.name) {
            return Ok(workflow_error_value(&WorkflowError::SystemDefinitionReadOnly(
                definition.id.clone(),
            )));
        }
        let source = match params.get("source").and_then(Value::as_str) {
            None => DefinitionSource::UserImported,
            Some(raw) => match DefinitionSource::parse(raw) {
                Some(DefinitionSource::System) | None => {
                    return Err(RPCErrors::NoPermission(format!(
                        "clients cannot submit definitions with source `{raw}`"
                    )));
                }
                Some(source) => source,
            },
        };
        let tags = params
            .get("tags")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let (report, _ctx) = analyze_workflow(&definition);
        if report.has_errors() {
            return Ok(json!({
                "ok": false,
                "error": "analysis_failed",
                "analysis": report,
            }));
        }
        let compiled = match compile_workflow_with(definition.clone(), self.orchestrator.functions()) {
            Ok(output) => output.workflow,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        let analysis = merge_warnings(report, &compiled);

        let record = self
            .definitions
            .upsert_with_source(caller.owner, source, None, definition, compiled, analysis, tags)
            .await;

        Ok(json!({
            "ok": true,
            "workflow_id": record.id,
            "version": record.version,
            "revision": record.compiled.revision,
            "source": record.source,
            "analysis": record.analysis,
            "definition": record.to_value(),
        }))
    }

    async fn get_definition(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let id = require_string(params, "workflow_id")?;
        match self.definitions.get_by_id(&id).await {
            Some(record) => {
                self.require_definition_access(&caller, &record)?;
                Ok(json!({ "ok": true, "definition": record.to_value() }))
            }
            None => Ok(not_found("workflow", &id)),
        }
    }

    async fn list_definitions(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let owner = optional_owner(params);
        if let Some(requested) = owner.as_ref() {
            if !caller.may_access(requested) {
                return Err(RPCErrors::NoPermission(
                    "cannot list another owner's definitions".to_string(),
                ));
            }
        }
        let status = params
            .get("status")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_value::<DefinitionStatus>(json!(s)).ok());
        let source = params
            .get("source")
            .and_then(Value::as_str)
            .and_then(DefinitionSource::parse);
        let tag = params
            .get("tag")
            .and_then(Value::as_str)
            .map(str::to_string);
        let records = self
            .definitions
            .list_filtered(owner.as_ref(), status, tag.as_deref(), source)
            .await;
        Ok(json!({
            "ok": true,
            "definitions": records
                .iter()
                .filter(|record| record.is_system() || caller.may_access(&record.owner))
                .map(|record| record.to_summary_value())
                .collect::<Vec<_>>(),
        }))
    }

    async fn archive_definition(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let id = require_string(params, "workflow_id")?;
        let Some(record) = self.definitions.get_by_id(&id).await else {
            return Ok(not_found("workflow", &id));
        };
        if record.is_system() {
            return Ok(workflow_error_value(&WorkflowError::SystemDefinitionReadOnly(id)));
        }
        self.require_definition_access(&caller, &record)?;
        match self.definitions.archive(&id).await {
            Some(record) => Ok(json!({
                "ok": true,
                "workflow_id": record.id,
                "status": record.status,
            })),
            None => Ok(not_found("workflow", &id)),
        }
    }

    async fn dry_run(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        self.caller(ctx).await?;
        let definition = require_definition(params)?;
        let (report, _ctx) = analyze_workflow(&definition);
        if report.has_errors() {
            return Ok(json!({
                "ok": false,
                "error": "analysis_failed",
                "analysis": report,
            }));
        }
        let compiled = match compile_workflow_with(definition, self.orchestrator.functions()) {
            Ok(output) => output.workflow,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        let merged = merge_warnings(report, &compiled);
        Ok(json!({
            "ok": true,
            "analysis": merged,
            "graph": compiled.graph,
            "revision": compiled.revision,
            "resolved_functions": compiled.resolved_functions,
        }))
    }

    async fn list_functions(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        self.caller(ctx).await?;
        let prefix = params.get("prefix").and_then(Value::as_str).unwrap_or("");
        let functions = self
            .orchestrator
            .functions()
            .list()
            .into_iter()
            .filter(|f| f.registration.name.starts_with(prefix))
            .map(|f| f.to_value())
            .collect::<Vec<_>>();
        Ok(json!({ "ok": true, "functions": functions }))
    }

    async fn get_function(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        self.caller(ctx).await?;
        let registry = self.orchestrator.functions();
        let found = if let Some(fun_id) = params.get("fun_id").and_then(Value::as_str) {
            registry.get(fun_id)
        } else {
            let name = require_string(params, "name")?;
            let version = params
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("1");
            registry.resolve(&name, version)
        };
        match found {
            Some(function) => {
                let stored = self
                    .object_store
                    .get_json(&function.registration.fun_id.to_string())
                    .await
                    .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
                let mut value = function.to_value();
                value["published"] = Value::Bool(stored.is_some());
                Ok(json!({ "ok": true, "function": value }))
            }
            None => Ok(not_found("function", "")),
        }
    }

    async fn get_capabilities(&self, ctx: &RPCContext) -> RpcResult<Value> {
        self.caller(ctx).await?;
        let env = self.orchestrator.environment();
        Ok(json!({
            "ok": true,
            "execution_epoch": self.execution_epoch,
            "environment": env.describe(),
            "baseline": crate::functions::media::media_tool_baseline(),
            "functions": self.orchestrator.functions().list().len(),
        }))
    }

    // ----- §3.2 Workflow Run 生命周期 ------------------------------------

    /// Resolves the definition by record id or by logical id / name of a
    /// system definition.
    async fn resolve_definition(&self, workflow_id: &str) -> Option<Arc<DefinitionRecord>> {
        if let Some(record) = self.definitions.get_by_id(workflow_id).await {
            return Some(record);
        }
        self.definitions
            .list_filtered(None, None, None, Some(DefinitionSource::System))
            .await
            .into_iter()
            .find(|record| record.name == workflow_id)
    }

    /// Normalizes the run input: schema defaults, validation, server-side
    /// resolution of `obj_id` to its content id. Callers never set
    /// `content_id` themselves.
    async fn resolve_trigger(
        &self,
        caller: &WorkflowCaller,
        ctx: &RPCContext,
        definition: &DefinitionRecord,
        input: Value,
    ) -> RpcResult<std::result::Result<Value, WorkflowError>> {
        let Some(schema) = trigger_input_schema(&definition.definition) else {
            return Ok(Ok(input));
        };
        let mut input = if input.is_null() { json!({}) } else { input };
        if let Some(map) = input.as_object_mut() {
            map.remove("content_id");
            map.remove("size");
        }
        apply_schema_defaults(&schema, &mut input);
        if let Err(err) = validate_json_schema(&schema, &input) {
            return Ok(Err(WorkflowError::InvalidInput(err)));
        }
        let Some(obj_id) = input.get("obj_id").and_then(Value::as_str).map(str::to_string) else {
            return Ok(Ok(input));
        };
        self.input_authorizer
            .authorize_read(caller, ctx.token.as_deref(), &obj_id)
            .await?;
        let file = match self.object_store.get_json(&obj_id).await {
            Ok(Some(value)) => value,
            Ok(None) => {
                return Ok(Err(WorkflowError::InvalidInput(format!(
                    "input object `{obj_id}` not found"
                ))))
            }
            Err(err) => return Ok(Err(err)),
        };
        let content_id = file
            .get("content")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        let Some(content_id) = content_id else {
            return Ok(Err(WorkflowError::InvalidInput(format!(
                "input object `{obj_id}` is not a file object with content"
            ))));
        };
        let map = input.as_object_mut().expect("validated object");
        map.insert("content_id".to_string(), Value::String(content_id));
        map.insert(
            "size".to_string(),
            file.get("size").cloned().unwrap_or(Value::from(0u64)),
        );
        if let Some(name) = file.get("name").cloned() {
            map.insert("name".to_string(), name);
        }
        if let Some(output_schema) = trigger_output_schema(&definition.definition) {
            if let Err(err) = validate_json_schema(&output_schema, &input) {
                return Ok(Err(WorkflowError::InvalidInput(err)));
            }
        }
        Ok(Ok(input))
    }

    async fn create_run(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller_matching_owner(params, ctx).await?;
        let workflow_id = require_string(params, "workflow_id")?;
        let trigger_input = params.get("input").cloned().unwrap_or(Value::Null);
        let callback_url = params
            .get("callback_url")
            .and_then(Value::as_str)
            .map(str::to_string);
        let auto_start = params
            .get("auto_start")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let ensure = params
            .get("ensure")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let wait_ms = params
            .get("wait_ms")
            .and_then(Value::as_u64)
            .unwrap_or(CREATE_RUN_WAIT_MS)
            .min(5_000);

        let definition = match self.resolve_definition(&workflow_id).await {
            Some(record) => record,
            None => return Ok(not_found("workflow", &workflow_id)),
        };
        self.require_definition_access(&caller, &definition)?;
        if definition.status == DefinitionStatus::Archived {
            return Ok(json!({
                "ok": false,
                "error": "definition_archived",
                "workflow_id": workflow_id,
            }));
        }

        let trigger_output = match self
            .resolve_trigger(&caller, ctx, &definition, trigger_input.clone())
            .await?
        {
            Ok(value) => value,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        let environment = self.orchestrator.environment();
        let ensure_key = ensure.then(|| {
            let payload = json!({
                "principal": caller.principal(),
                "definition_revision": definition.compiled.revision,
                "input": trigger_output,
                "environment_revision": environment.revision,
            });
            let (id, _) = ndn_lib::build_named_object_by_json("ensure", &payload);
            hex::encode(&id.obj_hash)
        });

        let mut initial_metrics = BTreeMap::new();
        if !trigger_input.is_null() {
            initial_metrics.insert("trigger_input".to_string(), trigger_input);
        }
        initial_metrics.insert(
            "environment_revision".to_string(),
            Value::String(environment.revision.clone()),
        );

        let (run, mut events) = match self
            .orchestrator
            .create_run_with_options(
                &definition.compiled,
                initial_metrics,
                RunOptions {
                    requested_by: Some(caller.principal()),
                    execution_epoch: self.execution_epoch,
                    trigger_output: Some(trigger_output),
                    deadline_at: None,
                },
            )
            .await
        {
            Ok(pair) => pair,
            Err(err) => return Ok(workflow_error_value(&err)),
        };

        let mut record = RunRecord {
            run,
            workflow_id: definition.id.clone(),
            owner: caller.owner.clone(),
            ensure_key: ensure_key.clone(),
            events: Vec::new(),
            amendments: Vec::new(),
            callback_url,
        };
        record.append_events(&events);
        let (handle, reused) = match self.runs.insert_or_ensure(record).await {
            EnsureOutcome::Created(handle) => (handle, false),
            EnsureOutcome::Reused(handle) => (handle, true),
        };
        let run_id = handle.state.lock().await.run.run_id.clone();
        if !reused {
            // Runs that can never wait for a human (system media pipelines)
            // need no TaskMgr human-action subscription (TODO F13).
            if let Some(subs) = self.subscriptions.as_ref() {
                if definition_accepts_human_actions(&definition) {
                    subs.watch_run(&run_id).await;
                }
            }
            if auto_start {
                self.driver.spawn_tick(run_id.clone());
                let _ = self
                    .driver
                    .wait_terminal(&run_id, std::time::Duration::from_millis(wait_ms))
                    .await;
            }
        }
        let record = handle.state.lock().await;
        if !reused {
            events = record.events.clone();
        } else {
            events.clear();
        }
        Ok(json!({
            "ok": true,
            "run_id": record.run.run_id,
            "workflow_id": handle.workflow_id,
            "status": record.run.status,
            "revision": record.run.definition_revision,
            "reused": reused,
            "output": record.run.output,
            "events": events,
            "seq": record.run.seq,
        }))
    }

    async fn start_run(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let wait_ms = params
            .get("wait_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(5_000);
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
        self.require_run_access(&caller, &handle)?;
        let (pre_seq, terminal) = {
            let record = handle.state.lock().await;
            (record.run.seq, record.run.is_terminal())
        };
        if terminal {
            return Ok(workflow_error_value(&WorkflowError::RunTerminal(run_id)));
        }
        self.driver.spawn_tick(run_id.clone());
        if wait_ms > 0 {
            let _ = self
                .driver
                .wait_terminal(&run_id, std::time::Duration::from_millis(wait_ms))
                .await;
        }
        let record = handle.state.lock().await;
        Ok(json!({
            "ok": true,
            "accepted": true,
            "run_id": record.run.run_id,
            "status": record.run.status,
            "events": record.events_since(pre_seq, 500),
            "from_seq": pre_seq,
            "to_seq": record.run.seq,
        }))
    }

    async fn tick_run(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        self.start_run(params, ctx).await
    }

    async fn cancel_run(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let reason = params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (handle, definition) = match self.lookup_run(&run_id).await {
            Ok(pair) => pair,
            Err(payload) => return Ok(payload),
        };
        self.require_run_access(&caller, &handle)?;
        let result = {
            let mut record = handle.state.lock().await;
            let pre_seq = record.run.seq;
            match self
                .orchestrator
                .cancel_run(&definition.compiled, &mut record.run, &caller.principal(), reason)
                .await
            {
                Ok(events) => {
                    record.append_events(&events);
                    Ok(json!({
                        "ok": true,
                        "run_id": record.run.run_id,
                        "status": record.run.status,
                        "events": events,
                        "from_seq": pre_seq,
                        "to_seq": record.run.seq,
                    }))
                }
                Err(err) => Err(err),
            }
        };
        match result {
            Ok(value) => {
                let _ = self.driver.tick_now(&run_id).await;
                Ok(value)
            }
            Err(err) => Ok(workflow_error_value(&err)),
        }
    }

    async fn get_run_graph(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let (handle, definition) = match self.lookup_run(&run_id).await {
            Ok(pair) => pair,
            Err(payload) => return Ok(payload),
        };
        self.require_run_access(&caller, &handle)?;
        let record = handle.state.lock().await;
        Ok(json!({
            "ok": true,
            "run_id": record.run.run_id,
            "workflow_id": handle.workflow_id,
            "workflow_name": record.run.workflow_name,
            "status": record.run.status,
            "revision": record.run.definition_revision,
            "requested_by": record.run.requested_by,
            "executor": "workflow",
            "owner": handle.owner.to_value(),
            "graph": definition.compiled.graph,
            "nodes": definition.compiled.nodes,
            "node_states": record.run.node_states,
            "node_outputs": record.run.node_outputs,
            "node_errors": record.run.node_errors,
            "node_sources": record.run.node_sources,
            "node_attempts": record.run.node_attempts,
            "human_waiting_nodes": record.run.human_waiting_nodes,
            "pending_thunks": record.run.pending_thunks,
            "metrics": record.run.metrics,
            "output": record.run.output,
            "deadline_at": record.run.deadline_at,
            "execution_epoch": record.run.execution_epoch,
            "created_at": record.run.created_at,
            "updated_at": record.run.updated_at,
            "seq": record.run.seq,
        }))
    }

    /// Terminal-node outputs plus the delivery of every artifact they
    /// reference (TODO D15): stable object ids in the output, access facts in
    /// a separate `delivery` block.
    async fn get_run_output(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
        self.require_run_access(&caller, &handle)?;
        let record = handle.state.lock().await;
        let run_error = record.run.metrics.get("run_error").cloned();
        let node_errors = record.run.node_errors.clone();
        let mut artifacts = Vec::new();
        for value in record.run.output.values() {
            collect_obj_ids(value, &mut artifacts);
        }
        artifacts.sort();
        artifacts.dedup();
        let output = record.run.output.clone();
        let (run_id, status, terminal, revision) = (
            record.run.run_id.clone(),
            record.run.status,
            record.run.is_terminal(),
            record.run.definition_revision.clone(),
        );
        drop(record);
        let expires_at = Utc::now().timestamp() + DELIVERY_TTL_SECS;
        let delivered = match self.orchestrator.io() {
            Some(io) => io.deliver(&caller.principal(), &artifacts, expires_at).await,
            None => Vec::new(),
        };
        Ok(json!({
            "ok": true,
            "run_id": run_id,
            "status": status,
            "terminal": terminal,
            "revision": revision,
            "output": output,
            "node_errors": node_errors,
            "run_error": run_error,
            "delivery": {
                "artifacts": artifacts,
                "grants": delivered,
                "principal": caller.principal(),
                "expires_at": expires_at,
            },
        }))
    }

    async fn list_runs(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let owner = optional_owner(params);
        if let Some(requested) = owner.as_ref() {
            if !caller.may_access(requested) {
                return Err(RPCErrors::NoPermission(
                    "cannot list another owner's runs".to_string(),
                ));
            }
        }
        let workflow_id = params
            .get("workflow_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let status = params
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_string);
        let handles = self.runs.list(owner.as_ref(), workflow_id.as_deref()).await;

        let mut out = Vec::with_capacity(handles.len());
        for handle in handles {
            if !caller.may_access(&handle.owner) {
                continue;
            }
            let record = handle.state.lock().await;
            if let Some(want) = status.as_ref() {
                if record.run.status.to_string().to_lowercase() != want.to_lowercase() {
                    continue;
                }
            }
            out.push(json!({
                "run_id": record.run.run_id,
                "workflow_id": handle.workflow_id,
                "workflow_name": record.run.workflow_name,
                "status": record.run.status,
                "revision": record.run.definition_revision,
                "owner": handle.owner.to_value(),
                "requested_by": record.run.requested_by,
                "created_at": record.run.created_at,
                "updated_at": record.run.updated_at,
                "seq": record.run.seq,
                "progress": record.run.progress_percent(),
            }));
        }
        Ok(json!({ "ok": true, "runs": out }))
    }

    // ----- §3.4 Agent / 外部系统集成 -------------------------------------

    async fn submit_step_output(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let node_id = require_string(params, "node_id")?;
        let actor = optional_actor(params);
        let output = params.get("output").cloned().unwrap_or(Value::Null);

        let (handle, definition) = match self.lookup_run(&run_id).await {
            Ok(pair) => pair,
            Err(payload) => return Ok(payload),
        };
        self.require_run_access(&caller, &handle)?;
        let result = {
            let mut record = handle.state.lock().await;
            let pre_seq = record.run.seq;
            match self
                .orchestrator
                .submit_step_output(
                    &definition.compiled,
                    &mut record.run,
                    &node_id,
                    &actor,
                    output,
                )
                .await
            {
                Ok(events) => {
                    record.append_events(&events);
                    Ok(pre_seq)
                }
                Err(err) => Err(err),
            }
        };
        let pre_seq = match result {
            Ok(seq) => seq,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        if let Err(err) = self.driver.tick_now(&run_id).await {
            return Ok(workflow_error_value(&err));
        }
        let record = handle.state.lock().await;
        Ok(json!({
            "ok": true,
            "run_id": record.run.run_id,
            "status": record.run.status,
            "events": record.events_since(pre_seq, 500),
            "from_seq": pre_seq,
            "to_seq": record.run.seq,
        }))
    }

    async fn report_step_progress(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let node_id = require_string(params, "node_id")?;
        let actor = optional_actor(params);
        let progress = params.get("progress").cloned().unwrap_or(Value::Null);

        let (handle, definition) = match self.lookup_run(&run_id).await {
            Ok(pair) => pair,
            Err(payload) => return Ok(payload),
        };
        self.require_run_access(&caller, &handle)?;
        let mut record = handle.state.lock().await;
        let pre_seq = record.run.seq;
        let events = match self
            .orchestrator
            .report_step_progress(
                &definition.compiled,
                &mut record.run,
                &node_id,
                &actor,
                progress,
            )
            .await
        {
            Ok(events) => events,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        record.append_events(&events);
        Ok(json!({
            "ok": true,
            "run_id": record.run.run_id,
            "events": events,
            "from_seq": pre_seq,
            "to_seq": record.run.seq,
        }))
    }

    async fn request_human(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let node_id = require_string(params, "node_id")?;
        let actor = optional_actor(params);
        let prompt = params
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::to_string);
        let subject = params.get("subject").cloned();

        let (handle, definition) = match self.lookup_run(&run_id).await {
            Ok(pair) => pair,
            Err(payload) => return Ok(payload),
        };
        self.require_run_access(&caller, &handle)?;
        let mut record = handle.state.lock().await;
        let pre_seq = record.run.seq;
        let events = match self
            .orchestrator
            .request_human(
                &definition.compiled,
                &mut record.run,
                &node_id,
                &actor,
                prompt,
                subject,
            )
            .await
        {
            Ok(events) => events,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        record.append_events(&events);
        Ok(json!({
            "ok": true,
            "run_id": record.run.run_id,
            "status": record.run.status,
            "events": events,
            "from_seq": pre_seq,
            "to_seq": record.run.seq,
        }))
    }

    // ----- §3.4 Amendment ------------------------------------------------

    async fn submit_amendment(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let patch = params.get("patch").cloned().unwrap_or(Value::Null);
        let actor = optional_actor(params);
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
        self.require_run_access(&caller, &handle)?;
        let mut record = handle.state.lock().await;
        let amendment = AmendmentRecord {
            id: format!("amend-{}", Uuid::new_v4()),
            plan_version: record.run.plan_version,
            patch,
            status: AmendmentStatus::Pending,
            submitted_by: actor,
            submitted_at: Utc::now().timestamp(),
            decided_by: None,
            decided_at: None,
            reason: None,
        };
        let payload = amendment.to_value();
        record.amendments.push(amendment);
        Ok(json!({
            "ok": true,
            "run_id": run_id,
            "amendment": payload,
        }))
    }

    async fn approve_amendment(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        self.decide_amendment(params, AmendmentStatus::Approved, ctx)
            .await
    }

    async fn reject_amendment(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        self.decide_amendment(params, AmendmentStatus::Rejected, ctx)
            .await
    }

    async fn decide_amendment(
        &self,
        params: &Value,
        status: AmendmentStatus,
        ctx: &RPCContext,
    ) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let amendment_id = require_string(params, "amendment_id")?;
        let actor = optional_actor(params);
        let reason = params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string);
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
        self.require_run_access(&caller, &handle)?;
        let mut record = handle.state.lock().await;
        let payload = {
            let amendment = match record.amendments.iter_mut().find(|a| a.id == amendment_id) {
                Some(a) => a,
                None => return Ok(not_found("amendment", &amendment_id)),
            };
            if amendment.status != AmendmentStatus::Pending {
                return Ok(json!({
                    "ok": false,
                    "error": "amendment_already_decided",
                    "status": amendment.status,
                }));
            }
            amendment.status = status;
            amendment.decided_by = Some(actor);
            amendment.decided_at = Some(Utc::now().timestamp());
            amendment.reason = reason;
            amendment.to_value()
        };
        if status == AmendmentStatus::Approved {
            // plan_version is the in-run amendment version; the definition
            // revision never moves (TODO D12).
            record.run.plan_version += 1;
            record.run.updated_at = Utc::now().timestamp();
        }
        Ok(json!({
            "ok": true,
            "amendment": payload,
            "plan_version": record.run.plan_version,
        }))
    }

    // ----- §3.5 事件订阅 -------------------------------------------------

    async fn get_history(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller(ctx).await?;
        let run_id = require_string(params, "run_id")?;
        let since_seq = params.get("since_seq").and_then(Value::as_u64).unwrap_or(0);
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
        self.require_run_access(&caller, &handle)?;
        let record = handle.state.lock().await;
        let events = record.events_since(since_seq, limit);
        let next_seq = events.last().map(|e| e.seq).unwrap_or(since_seq);
        Ok(json!({
            "ok": true,
            "run_id": run_id,
            "status": record.run.status,
            "events": events,
            "next_seq": next_seq,
            "current_seq": record.run.seq,
        }))
    }

    async fn subscribe_events(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let run_id = require_string(params, "run_id")?;
        let history_payload = self.get_history(params, ctx).await?;
        Ok(json!({
            "ok": true,
            "channel": format!("workflow.events.{}", run_id),
            "transport": "kmsgqueue",
            "history": history_payload,
        }))
    }

    // ----- Schedule / Trigger -------------------------------------------

    async fn create_scheduled_task(&self, params: &Value, owner: Owner) -> RpcResult<Value> {
        let name = require_string(params, "name")?;
        let description = params
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string);
        let schedule = parse_schedule_spec(params)?;
        let target = parse_schedule_target(params)?;
        self.validate_target_exists(&target).await?;
        validate_subtask_template(&target)
            .map_err(|err| RPCErrors::ParseRequestError(format!("invalid `target`: {}", err)))?;
        let policy = schedule_policy_from_value(params.get("policy"))
            .map_err(|err| RPCErrors::ParseRequestError(format!("invalid `policy`: {}", err)))?;
        let now = Utc::now().timestamp();
        let status = optional_schedule_status(params).unwrap_or(ScheduleStatus::Running);
        let next_fire_at = if status == ScheduleStatus::Running {
            initial_next_fire_at(&schedule, now)
        } else {
            None
        };
        let record = WorkflowSchedule {
            schedule_id: format!("sch-{}", Uuid::new_v4()),
            owner,
            name,
            description,
            status,
            schedule,
            target,
            state: ScheduleState {
                next_fire_at,
                ..Default::default()
            },
            policy,
            task_mirror: ScheduleTaskMirror::default(),
            created_at: now,
            updated_at: now,
        };
        let mut record = self.schedules.insert(record).await;
        record = self.ensure_schedule_root_task(&record).await?;
        Ok(json!({
            "ok": true,
            "schedule_id": record.schedule_id,
            "schedule": record.to_value(),
        }))
    }

    async fn authenticated_schedule_owner(
        &self,
        params: &Value,
        ctx: &RPCContext,
    ) -> RpcResult<Owner> {
        let requested = require_owner(params)?;
        let authenticated = self.caller_verifier.verify(ctx).await?.owner;
        if requested != authenticated {
            return Err(RPCErrors::NoPermission(format!(
                "schedule owner {}:{} does not match authenticated caller {}:{}",
                requested.user_id, requested.app_id, authenticated.user_id, authenticated.app_id
            )));
        }
        Ok(authenticated)
    }

    async fn update_scheduled_task(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let schedule_id = require_string(params, "schedule_id")?;
        let mut next_schedule = match self.schedule_for_caller(&schedule_id, ctx).await? {
            Some(record) => record,
            None => return Ok(not_found("schedule", &schedule_id)),
        };
        if let Some(name) = params.get("name").and_then(Value::as_str) {
            next_schedule.name = name.to_string();
        }
        if params.get("description").is_some() {
            next_schedule.description = params
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if params.get("schedule").is_some() {
            next_schedule.schedule = parse_schedule_spec(params)?;
            if next_schedule.status == ScheduleStatus::Running {
                next_schedule.state.next_fire_at =
                    initial_next_fire_at(&next_schedule.schedule, Utc::now().timestamp());
            }
        }
        if params.get("target").is_some() {
            next_schedule.target = parse_schedule_target(params)?;
            self.validate_target_exists(&next_schedule.target).await?;
            validate_subtask_template(&next_schedule.target).map_err(|err| {
                RPCErrors::ParseRequestError(format!("invalid `target`: {}", err))
            })?;
        }
        if params.get("policy").is_some() {
            next_schedule.policy =
                schedule_policy_from_value(params.get("policy")).map_err(|err| {
                    RPCErrors::ParseRequestError(format!("invalid `policy`: {}", err))
                })?;
        }

        let updated = self
            .schedules
            .update(&schedule_id, |record| {
                *record = next_schedule.clone();
            })
            .await
            .unwrap();
        self.update_scheduled_task_root_task(&updated).await;
        Ok(json!({ "ok": true, "schedule": updated.to_value() }))
    }

    async fn get_scheduled_task(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let schedule_id = require_string(params, "schedule_id")?;
        match self.schedule_for_caller(&schedule_id, ctx).await? {
            Some(record) => Ok(json!({ "ok": true, "schedule": record.to_value() })),
            None => Ok(not_found("schedule", &schedule_id)),
        }
    }

    async fn list_scheduled_tasks(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let caller = self.caller_verifier.verify(ctx).await?.owner;
        if let Some(requested) = optional_owner(params) {
            if requested != caller {
                return Err(RPCErrors::NoPermission(
                    "cannot list another owner's schedules".to_string(),
                ));
            }
        }
        let status = params
            .get("status")
            .and_then(Value::as_str)
            .and_then(parse_schedule_status);
        let workflow_id = params.get("workflow_id").and_then(Value::as_str);
        let name = params.get("name").and_then(Value::as_str);
        let records = self
            .schedules
            .list(Some(&caller), status, workflow_id, name)
            .await;
        Ok(json!({
            "ok": true,
            "schedules": records.iter().map(WorkflowSchedule::to_summary_value).collect::<Vec<_>>(),
        }))
    }

    async fn pause_scheduled_task(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        self.set_schedule_status(params, ScheduleStatus::Paused, ctx)
            .await
    }

    async fn resume_scheduled_task(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let schedule_id = require_string(params, "schedule_id")?;
        if self.schedule_for_caller(&schedule_id, ctx).await?.is_none() {
            return Ok(not_found("schedule", &schedule_id));
        }
        let updated = self
            .schedules
            .update(&schedule_id, |record| {
                record.status = ScheduleStatus::Running;
                record.state.next_fire_at =
                    initial_next_fire_at(&record.schedule, Utc::now().timestamp());
                record.state.last_error = None;
            })
            .await;
        match updated {
            Some(record) => {
                self.update_scheduled_task_root_task(&record).await;
                Ok(json!({ "ok": true, "schedule": record.to_value() }))
            }
            None => Ok(not_found("schedule", &schedule_id)),
        }
    }

    async fn archive_scheduled_task(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        self.set_schedule_status(params, ScheduleStatus::Canceled, ctx)
            .await
    }

    async fn set_schedule_status(
        &self,
        params: &Value,
        status: ScheduleStatus,
        ctx: &RPCContext,
    ) -> RpcResult<Value> {
        let schedule_id = require_string(params, "schedule_id")?;
        if self.schedule_for_caller(&schedule_id, ctx).await?.is_none() {
            return Ok(not_found("schedule", &schedule_id));
        }
        let updated = self
            .schedules
            .update(&schedule_id, |record| {
                record.status = status;
                if status == ScheduleStatus::Canceled {
                    record.state.next_fire_at = None;
                }
            })
            .await;
        match updated {
            Some(record) => {
                self.update_scheduled_task_root_task(&record).await;
                Ok(json!({ "ok": true, "schedule": record.to_value() }))
            }
            None => Ok(not_found("schedule", &schedule_id)),
        }
    }

    async fn run_scheduled_task_now(&self, params: &Value, ctx: &RPCContext) -> RpcResult<Value> {
        let schedule_id = require_string(params, "schedule_id")?;
        if self.schedule_for_caller(&schedule_id, ctx).await?.is_none() {
            return Ok(not_found("schedule", &schedule_id));
        }
        let fire_time = params
            .get("fire_time")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| Utc::now().timestamp());
        match self.fire_schedule(&schedule_id, fire_time, true).await {
            Ok(fire) => Ok(json!({
                "ok": true,
                "fire_id": fire.fire_id,
                "task_id": fire.task_id,
                "run_id": fire.run_id,
                "fire": fire.to_value()
            })),
            Err(payload) => Ok(payload),
        }
    }

    async fn get_scheduled_task_history(
        &self,
        params: &Value,
        ctx: &RPCContext,
    ) -> RpcResult<Value> {
        let schedule_id = require_string(params, "schedule_id")?;
        if self.schedule_for_caller(&schedule_id, ctx).await?.is_none() {
            return Ok(not_found("schedule", &schedule_id));
        }
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
        let history = self.schedules.history(&schedule_id, limit).await;
        Ok(json!({
            "ok": true,
            "schedule_id": schedule_id,
            "fires": history.iter().map(ScheduleFireRecord::to_value).collect::<Vec<_>>(),
        }))
    }

    async fn schedule_for_caller(
        &self,
        schedule_id: &str,
        ctx: &RPCContext,
    ) -> RpcResult<Option<WorkflowSchedule>> {
        let caller = self.caller_verifier.verify(ctx).await?.owner;
        let Some(schedule) = self.schedules.get(schedule_id).await else {
            return Ok(None);
        };
        if schedule.owner != caller {
            return Err(RPCErrors::NoPermission(format!(
                "caller {}:{} may not access schedule {}",
                caller.user_id, caller.app_id, schedule_id
            )));
        }
        Ok(Some(schedule))
    }

    async fn validate_scheduled_task(&self, params: &Value) -> RpcResult<Value> {
        let schedule = parse_schedule_spec(params)?;
        if params.get("target").is_some() {
            let target = parse_schedule_target(params)?;
            self.validate_target_exists(&target).await?;
            validate_subtask_template(&target).map_err(|err| {
                RPCErrors::ParseRequestError(format!("invalid `target`: {}", err))
            })?;
        }
        let times = next_fire_times(&schedule, Utc::now().timestamp(), 3);
        let normalized_expr = match &schedule {
            ScheduleSpec::Cron { expr, .. } => Some(expr.clone()),
            ScheduleSpec::Once { .. } => None,
            ScheduleSpec::RunEvery { .. } => None,
        };
        let timezone = match &schedule {
            ScheduleSpec::Cron { timezone, .. } => timezone.clone(),
            ScheduleSpec::Once { timezone, .. } => timezone.clone().unwrap_or_else(|| "UTC".into()),
            ScheduleSpec::RunEvery { timezone, .. } => {
                timezone.clone().unwrap_or_else(|| "UTC".into())
            }
        };
        Ok(json!({
            "ok": true,
            "valid": true,
            "normalized_expr": normalized_expr,
            "timezone": timezone,
            "next_fire_times": times.iter().map(|ts| rfc3339(*ts)).collect::<Vec<_>>(),
            "warnings": [],
        }))
    }

    /// 把内存投影从 Task DB（唯一真相源）灌入。boot 时 task_mgr 可能未就绪 ⇒
    /// 加载失败时不放弃，保持未 hydrate，由 scan 循环下个 tick 重试（sweep 兜底）。
    /// 成功一次即停；hydrate 本身 insert-if-absent，不覆盖进程内已有的运行态。
    async fn ensure_schedules_hydrated(&self) {
        use std::sync::atomic::Ordering;
        if self.schedules_hydrated.load(Ordering::Relaxed) {
            return;
        }
        let Some(mirror) = self.schedule_mirror.as_ref() else {
            // 没有 task_mgr（测试 / in-process）：无可加载，视为已 hydrate。
            self.schedules_hydrated.store(true, Ordering::Relaxed);
            return;
        };
        match mirror.load_schedules().await {
            Ok(loaded) => {
                let count = loaded.len();
                self.schedules.hydrate(loaded).await;
                self.schedules_hydrated.store(true, Ordering::Relaxed);
                log::info!("workflow schedules hydrated from task db: {}", count);
            }
            Err(err) => {
                if !self.schedules_hydrate_warned.swap(true, Ordering::Relaxed) {
                    log::warn!(
                        "hydrate workflow schedules from task db failed (will retry each scan tick): {}",
                        err
                    );
                } else {
                    log::debug!("hydrate workflow schedules retry failed: {}", err);
                }
            }
        }
    }

    pub async fn scan_due_schedules(&self) {
        let tick = self
            .scan_ticks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if tick % RUN_PRUNE_EVERY_TICKS == 0 {
            let pruned = self
                .runs
                .prune_terminal(MAX_TERMINAL_RUNS, TERMINAL_RUN_MAX_AGE_SECS)
                .await;
            if pruned > 0 {
                log::info!("workflow pruned {} terminal runs from memory", pruned);
            }
        }
        self.ensure_schedules_hydrated().await;
        if !self
            .schedules_hydrated
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            // task_mgr 尚不可达，schedule 未加载，跳过本 tick，下个 tick 再重试。
            return;
        }
        let now = Utc::now().timestamp();
        let due = self.schedules.due(now).await;
        for schedule in due {
            let (fire_times, next_fire_at, missed_error) = due_fire_times(&schedule, now);
            if let Some(error) = missed_error {
                let updated = self
                    .schedules
                    .update(&schedule.schedule_id, |record| {
                        record.state.last_error = Some(error);
                        record.state.next_fire_at = next_fire_at;
                    })
                    .await;
                if let Some(record) = updated.as_ref() {
                    self.update_scheduled_task_root_task(record).await;
                }
                continue;
            }
            for fire_time in fire_times {
                if let Err(payload) = self
                    .fire_schedule(&schedule.schedule_id, fire_time, false)
                    .await
                {
                    log::warn!("workflow schedule fire failed: {}", payload);
                }
            }
            if matches!(schedule.policy.misfire, MisfirePolicy::Skip) {
                if let Some(updated) = self
                    .schedules
                    .update(&schedule.schedule_id, |record| {
                        record.state.next_fire_at = next_fire_at;
                    })
                    .await
                {
                    self.update_scheduled_task_root_task(&updated).await;
                }
            }
        }
    }

    async fn fire_schedule(
        &self,
        schedule_id: &str,
        fire_time: i64,
        manual: bool,
    ) -> std::result::Result<ScheduleFireRecord, Value> {
        let schedule = self
            .schedules
            .get(schedule_id)
            .await
            .ok_or_else(|| not_found("schedule", schedule_id))?;
        if !manual && schedule.status != ScheduleStatus::Running {
            return Err(json!({
                "ok": false,
                "error": "schedule_not_enabled",
                "schedule_id": schedule_id,
            }));
        }
        let (fire, is_new) = self
            .schedules
            .begin_fire(&schedule.schedule_id, fire_time, manual)
            .await;
        if !is_new {
            return Ok(fire);
        }

        if schedule.task_mirror.root_task_id.is_none() {
            return Err(self
                .fail_schedule_fire(&schedule, &fire, "schedule_root_task_missing".to_string())
                .await);
        }

        let active = self.active_schedule_runs(&schedule).await;
        if active > 0 {
            let completed = self
                .schedules
                .complete_fire(
                    &fire.fire_id,
                    FireStatus::Skipped,
                    None,
                    None,
                    Some("previous_run_still_active".to_string()),
                )
                .await
                .unwrap_or(fire);
            let updated = self
                .schedules
                .update(&schedule.schedule_id, |record| {
                    record.state.last_fire_at = Some(fire_time);
                    record.state.last_error = Some("previous_run_still_active".to_string());
                    if !manual {
                        record.state.next_fire_at = next_fire_after(&record.schedule, fire_time);
                    }
                })
                .await;
            if let Some(record) = updated.as_ref() {
                self.update_scheduled_task_root_task(record).await;
            }
            return Ok(completed);
        }

        let rendered = render_subtask_template(&schedule, &fire);
        if rendered.task_type == "workflow.run" {
            let workflow_id = match rendered
                .data
                .pointer("/request/workflow_id")
                .and_then(Value::as_str)
            {
                Some(value) => value.to_string(),
                None => {
                    return Err(self
                        .fail_schedule_fire(
                            &schedule,
                            &fire,
                            "workflow_run_template_missing_workflow_id".to_string(),
                        )
                        .await);
                }
            };
            let input = rendered
                .data
                .pointer("/request/input")
                .cloned()
                .unwrap_or(Value::Null);
            let definition = match self.definitions.get_by_id(&workflow_id).await {
                Some(record) if record.status != DefinitionStatus::Archived => record,
                Some(_) => {
                    return Err(self
                        .fail_schedule_fire(&schedule, &fire, "definition_archived".to_string())
                        .await);
                }
                None => {
                    return Err(self
                        .fail_schedule_fire(&schedule, &fire, "workflow_not_found".to_string())
                        .await);
                }
            };
            let trigger = schedule_trigger_context(&schedule, fire_time, manual);
            let trigger_input = merge_trigger_input(input.clone(), trigger.clone());
            let mut metrics = BTreeMap::new();
            metrics.insert("trigger".to_string(), trigger);
            metrics.insert("trigger_input".to_string(), trigger_input);
            if let (Some(root_task_id), Some(root_id)) = (
                schedule.task_mirror.root_task_id.clone(),
                schedule.task_mirror.root_id.clone(),
            ) {
                metrics.insert(
                    "schedule_task".to_string(),
                    json!({
                        "root_task_id": root_task_id,
                        "root_id": root_id,
                    }),
                );
            }
            let (run, events) = match self
                .orchestrator
                .create_run_with_options(
                    &definition.compiled,
                    metrics,
                    RunOptions {
                        requested_by: Some(schedule.owner.principal()),
                        execution_epoch: self.execution_epoch,
                        trigger_output: Some(input),
                        deadline_at: None,
                    },
                )
                .await
            {
                Ok(pair) => pair,
                Err(err) => {
                    return Err(self
                        .fail_schedule_fire(&schedule, &fire, err.to_string())
                        .await);
                }
            };
            let run_id = run.run_id.clone();
            let mut record = RunRecord {
                run,
                workflow_id: definition.id.clone(),
                owner: schedule.owner.clone(),
                ensure_key: None,
                events: Vec::new(),
                amendments: Vec::new(),
                callback_url: None,
            };
            record.append_events(&events);
            let _ = self.runs.insert(record).await;
            if let Some(subs) = self.subscriptions.as_ref() {
                subs.watch_run(&run_id).await;
            }
            self.driver.spawn_tick(run_id.clone());
            let task_id = match self.schedule_mirror.as_ref() {
                Some(mirror) => {
                    match mirror.find_fire_subtask_by_run_id(&schedule, &run_id).await {
                        Ok(task_id) => task_id,
                        Err(err) => {
                            log::warn!("lookup workflow run fire subtask failed: {}", err);
                            None
                        }
                    }
                }
                None => None,
            };
            let completed = self
                .schedules
                .complete_fire(
                    &fire.fire_id,
                    FireStatus::TaskCreated,
                    task_id.clone(),
                    Some(run_id.clone()),
                    None,
                )
                .await
                .unwrap_or(fire);
            let updated = self
                .schedules
                .update(&schedule.schedule_id, |record| {
                    record.state.last_fire_at = Some(fire_time);
                    record.state.last_task_id = task_id.clone();
                    record.state.last_run_id = Some(run_id);
                    record.state.consecutive_failures = 0;
                    record.state.last_error = None;
                    if matches!(record.schedule, ScheduleSpec::Once { .. }) {
                        record.status = ScheduleStatus::Canceled;
                        record.state.next_fire_at = None;
                    } else if is_reboot_schedule(&record.schedule) {
                        record.state.next_fire_at = None;
                    } else if !manual {
                        record.state.next_fire_at = next_fire_after(&record.schedule, fire_time);
                    }
                })
                .await;
            if let Some(record) = updated.as_ref() {
                self.update_scheduled_task_root_task(record).await;
            }
            Ok(completed)
        } else {
            let Some(mirror) = self.schedule_mirror.as_ref() else {
                return Err(self
                    .fail_schedule_fire(&schedule, &fire, "task_manager_unavailable".to_string())
                    .await);
            };
            let task_id = match mirror
                .create_fire_subtask(&schedule, &fire, &rendered)
                .await
            {
                Ok(task_id) => task_id,
                Err(err) => {
                    return Err(self.fail_schedule_fire(&schedule, &fire, err).await);
                }
            };
            let completed = self
                .schedules
                .complete_fire(
                    &fire.fire_id,
                    FireStatus::TaskCreated,
                    Some(task_id.clone()),
                    None,
                    None,
                )
                .await
                .unwrap_or(fire);
            self.complete_schedule_after_success(&schedule, fire_time, manual, Some(task_id), None)
                .await;
            Ok(completed)
        }
    }

    async fn complete_schedule_after_success(
        &self,
        schedule: &WorkflowSchedule,
        fire_time: i64,
        manual: bool,
        task_id: Option<String>,
        run_id: Option<String>,
    ) {
        let updated = self
            .schedules
            .update(&schedule.schedule_id, |record| {
                record.state.last_fire_at = Some(fire_time);
                record.state.last_task_id = task_id.clone();
                record.state.last_run_id = run_id;
                record.state.consecutive_failures = 0;
                record.state.last_error = None;
                if matches!(record.schedule, ScheduleSpec::Once { .. }) {
                    record.status = ScheduleStatus::Canceled;
                    record.state.next_fire_at = None;
                } else if is_reboot_schedule(&record.schedule) {
                    record.state.next_fire_at = None;
                } else if !manual {
                    record.state.next_fire_at = next_fire_after(&record.schedule, fire_time);
                }
            })
            .await;
        if let Some(record) = updated.as_ref() {
            self.update_scheduled_task_root_task(record).await;
        }
    }

    async fn fail_schedule_fire(
        &self,
        schedule: &WorkflowSchedule,
        fire: &ScheduleFireRecord,
        error: String,
    ) -> Value {
        let _ = self
            .schedules
            .complete_fire(
                &fire.fire_id,
                FireStatus::Failed,
                None,
                None,
                Some(error.clone()),
            )
            .await;
        let updated = self
            .schedules
            .update(&schedule.schedule_id, |record| {
                record.status = ScheduleStatus::Failed;
                record.state.consecutive_failures =
                    record.state.consecutive_failures.saturating_add(1);
                record.state.last_error = Some(error.clone());
            })
            .await;
        if let Some(record) = updated.as_ref() {
            self.update_scheduled_task_root_task(record).await;
        }
        json!({
            "ok": false,
            "error": "schedule_fire_failed",
            "schedule_id": schedule.schedule_id,
            "message": error,
        })
    }

    async fn active_schedule_runs(&self, schedule: &WorkflowSchedule) -> u32 {
        if let Some(mirror) = self.schedule_mirror.as_ref() {
            match mirror.active_fire_subtasks(schedule).await {
                Ok(active) if active > 0 => return active,
                Ok(_) => {}
                Err(err) => log::warn!("query active schedule fire subtasks failed: {}", err),
            }
        }
        let handles = self.runs.list(None, None).await;
        let mut count = 0;
        for handle in handles {
            let record = handle.state.lock().await;
            if record
                .run
                .metrics
                .get("trigger")
                .and_then(|value| value.get("schedule_id"))
                .and_then(Value::as_str)
                == Some(schedule.schedule_id.as_str())
                && !record.run.is_terminal()
            {
                count += 1;
            }
        }
        count
    }

    async fn validate_target_exists(&self, target: &ScheduleTarget) -> RpcResult<()> {
        if let Some(workflow_id) = schedule_workflow_id(target) {
            match self.definitions.get_by_id(workflow_id).await {
                Some(record) if record.status != DefinitionStatus::Archived => Ok(()),
                Some(_) => Err(RPCErrors::ReasonError(format!(
                    "workflow `{}` is archived",
                    workflow_id
                ))),
                None => Err(RPCErrors::ReasonError(format!(
                    "workflow `{}` not found",
                    workflow_id
                ))),
            }
        } else {
            Ok(())
        }
    }

    async fn ensure_schedule_root_task(
        &self,
        schedule: &WorkflowSchedule,
    ) -> RpcResult<WorkflowSchedule> {
        let Some(mirror) = self.schedule_mirror.as_ref() else {
            return Err(RPCErrors::ReasonError(
                "task_manager_unavailable: enabled schedule requires a root task".to_string(),
            ));
        };
        match mirror.ensure_root_task(schedule).await {
            Ok(task_mirror) => self
                .schedules
                .update(&schedule.schedule_id, |record| {
                    record.task_mirror = task_mirror;
                })
                .await
                .ok_or_else(|| RPCErrors::ReasonError("schedule disappeared".to_string())),
            Err(err) => Err(RPCErrors::ReasonError(format!(
                "create schedule root task failed: {}",
                err
            ))),
        }
    }

    async fn update_scheduled_task_root_task(&self, schedule: &WorkflowSchedule) {
        if let Some(mirror) = self.schedule_mirror.as_ref() {
            if let Err(err) = mirror.update_root_task(schedule).await {
                log::warn!("workflow schedule task mirror update failed: {}", err);
            }
        }
    }

    // ----- 共用辅助 -------------------------------------------------

    async fn lookup_run(
        &self,
        run_id: &str,
    ) -> std::result::Result<(Arc<RunHandle>, Arc<DefinitionRecord>), Value> {
        let handle = match self.runs.get(run_id).await {
            Some(h) => h,
            None => return Err(not_found("run", run_id)),
        };
        let definition = match self.definitions.get_by_id(&handle.workflow_id).await {
            Some(d) => d,
            None => {
                let payload = not_found("workflow", &handle.workflow_id);
                return Err(payload);
            }
        };
        Ok((handle, definition))
    }
}

fn definition_accepts_human_actions(definition: &DefinitionRecord) -> bool {
    if !definition.is_system() {
        return true;
    }
    definition.definition.steps.iter().any(|step| {
        step.step_type != buckyos_api::workflow_dsl::StepType::Autonomous
            || step
                .guards
                .as_ref()
                .and_then(|g| g.retry.as_ref())
                .and_then(|r| r.fallback)
                .map(|f| f == buckyos_api::workflow_dsl::RetryFallback::Human)
                .unwrap_or(true)
    })
}

/// Collects every string that looks like a named object id (`type:hex`).
fn collect_obj_ids(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            if ndn_lib::ObjId::new(s).is_ok() && s.contains(':') {
                out.push(s.clone());
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_obj_ids(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_obj_ids(v, out)),
        _ => {}
    }
}

/// 把 `service.foo` 与裸 `foo` 都规整到同一个内部 case。
fn canonical_method(method: &str) -> &str {
    method
        .strip_prefix("service.")
        .or_else(|| method.strip_prefix("workflow."))
        .unwrap_or(method)
}

fn require_string(params: &Value, field: &str) -> RpcResult<String> {
    params
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RPCErrors::ParseRequestError(format!("missing required field `{}`", field)))
}

fn require_owner(params: &Value) -> RpcResult<Owner> {
    Owner::from_value(
        params
            .get("owner")
            .ok_or_else(|| RPCErrors::ParseRequestError("missing required field `owner`".into()))?,
    )
    .ok_or_else(|| RPCErrors::ParseRequestError("invalid `owner`".into()))
}

fn optional_owner(params: &Value) -> Option<Owner> {
    params.get("owner").and_then(Owner::from_value)
}

fn optional_actor(params: &Value) -> String {
    params
        .get("actor")
        .and_then(Value::as_str)
        .unwrap_or("agent")
        .to_string()
}

fn parse_schedule_spec(params: &Value) -> RpcResult<ScheduleSpec> {
    let raw = params
        .get("schedule")
        .ok_or_else(|| RPCErrors::ParseRequestError("missing required field `schedule`".into()))?;
    schedule_spec_from_value(raw)
        .map_err(|err| RPCErrors::ParseRequestError(format!("invalid `schedule`: {}", err)))
}

fn parse_schedule_target(params: &Value) -> RpcResult<ScheduleTarget> {
    let raw = params
        .get("target")
        .ok_or_else(|| RPCErrors::ParseRequestError("missing required field `target`".into()))?;
    schedule_target_from_value(raw)
        .map_err(|err| RPCErrors::ParseRequestError(format!("invalid `target`: {}", err)))
}

fn optional_schedule_status(params: &Value) -> Option<ScheduleStatus> {
    params
        .get("status")
        .and_then(Value::as_str)
        .and_then(parse_schedule_status)
}

fn parse_schedule_status(raw: &str) -> Option<ScheduleStatus> {
    ScheduleStatus::from_str_loose(raw)
}

fn initial_next_fire_at(schedule: &ScheduleSpec, now: i64) -> Option<i64> {
    match schedule {
        ScheduleSpec::Once { run_at, .. } if *run_at <= now => Some(*run_at),
        _ => next_fire_after(schedule, now),
    }
}

fn schedule_trigger_context(schedule: &WorkflowSchedule, fire_time: i64, manual: bool) -> Value {
    let (cron, timezone) = match &schedule.schedule {
        ScheduleSpec::Cron { expr, timezone, .. } => (Some(expr.clone()), Some(timezone.clone())),
        ScheduleSpec::Once { timezone, .. } => (None, timezone.clone()),
        ScheduleSpec::RunEvery { timezone, .. } => (None, timezone.clone()),
    };
    json!({
        "kind": "schedule",
        "schedule_id": schedule.schedule_id,
        "fire_time": rfc3339(fire_time),
        "fire_time_unix": fire_time,
        "cron": cron,
        "timezone": timezone.unwrap_or_else(|| "UTC".to_string()),
        "manual": manual,
    })
}

fn merge_trigger_input(input: Value, trigger: Value) -> Value {
    match input {
        Value::Object(mut map) => {
            map.insert("trigger".to_string(), trigger);
            Value::Object(map)
        }
        Value::Null => json!({ "trigger": trigger }),
        other => json!({ "input": other, "trigger": trigger }),
    }
}

fn require_definition(params: &Value) -> RpcResult<WorkflowDefinition> {
    let raw = params.get("definition").cloned().ok_or_else(|| {
        RPCErrors::ParseRequestError("missing required field `definition`".into())
    })?;
    serde_json::from_value::<WorkflowDefinition>(raw)
        .map_err(|err| RPCErrors::ParseRequestError(format!("invalid `definition`: {}", err)))
}

fn not_found(kind: &str, id: &str) -> Value {
    json!({
        "ok": false,
        "error": format!("{}_not_found", kind),
        "id": id,
    })
}

pub fn workflow_error_value(err: &WorkflowError) -> Value {
    let (code, message, detail) = workflow_error_payload(err);
    let mut payload = json!({
        "ok": false,
        "error": code,
        "message": message,
    });
    if let Some(detail) = detail {
        payload["detail"] = detail;
    }
    payload
}

fn merge_warnings(report: AnalysisReport, compiled: &CompiledWorkflow) -> AnalysisReport {
    let mut report = report;
    for warning in &compiled.warnings {
        if !report
            .warnings
            .iter()
            .any(|existing| existing.code == warning.code && existing.node_id == warning.node_id)
        {
            report.warnings.push(warning.clone());
        }
    }
    report
}

#[cfg(test)]
mod tests;
