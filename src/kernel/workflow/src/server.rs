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
use buckyos_api::WorkflowDefinition;
use chrono::Utc;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    analyze_workflow, compile_workflow, AnalysisReport, CompiledWorkflow, InMemoryObjectStore,
    InMemoryThunkDispatcher, WorkflowError, WorkflowOrchestrator,
};

use crate::state::{
    workflow_error_payload, AmendmentRecord, AmendmentStatus, DefinitionStatus, DefinitionStore,
    Owner, RunRecord, RunStore, ServiceTracker,
};

type RpcResult<T> = std::result::Result<T, RPCErrors>;

pub type ServiceOrchestrator =
    WorkflowOrchestrator<InMemoryThunkDispatcher, InMemoryObjectStore, ServiceTracker>;

/// 把 method dispatch + 各 method 的真正实现集中起来。
pub struct WorkflowRpcHandler {
    definitions: Arc<DefinitionStore>,
    runs: Arc<RunStore>,
    orchestrator: Arc<ServiceOrchestrator>,
}

impl WorkflowRpcHandler {
    pub fn new(
        definitions: Arc<DefinitionStore>,
        runs: Arc<RunStore>,
        orchestrator: Arc<ServiceOrchestrator>,
    ) -> Self {
        Self {
            definitions,
            runs,
            orchestrator,
        }
    }

    pub async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> RpcResult<RPCResponse> {
        let method = canonical_method(&req.method);

        let result = match method {
            // §3.1 Definition
            "submit_definition" => self.submit_definition(&req.params).await,
            "get_definition" => self.get_definition(&req.params).await,
            "list_definitions" => self.list_definitions(&req.params).await,
            "archive_definition" => self.archive_definition(&req.params).await,
            "dry_run" => self.dry_run(&req.params).await,
            // §3.2 Run lifecycle
            "create_run" => self.create_run(&req.params).await,
            "start_run" => self.start_run(&req.params).await,
            "tick_run" => self.tick_run(&req.params).await,
            "get_run_graph" => self.get_run_graph(&req.params).await,
            "list_runs" => self.list_runs(&req.params).await,
            // §3.4 Agent
            "submit_step_output" => self.submit_step_output(&req.params).await,
            "report_step_progress" => self.report_step_progress(&req.params).await,
            "request_human" => self.request_human(&req.params).await,
            // §3.4 Amendment
            "submit_amendment" => self.submit_amendment(&req.params).await,
            "approve_amendment" => self.approve_amendment(&req.params).await,
            "reject_amendment" => self.reject_amendment(&req.params).await,
            // §3.5 Events
            "get_history" => self.get_history(&req.params).await,
            "subscribe_events" => self.subscribe_events(&req.params).await,
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

    // ----- §3.1 Workflow Definition --------------------------------------

    async fn submit_definition(&self, params: &Value) -> RpcResult<Value> {
        let owner = require_owner(params)?;
        let definition = require_definition(params)?;
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

        // §3.1 / §10.5：dry_run 与 submit 必须走同一条 analyze + compile 流水。
        // 先 analyze 拿到完整 report（包含 warnings），有 error 直接拒绝；
        // 然后 compile（compile 的 warnings 与 analyze 一致，做防御性合并）。
        let (report, _ctx) = analyze_workflow(&definition);
        if report.has_errors() {
            return Ok(json!({
                "ok": false,
                "error": "analysis_failed",
                "analysis": report,
            }));
        }
        let compiled = match compile_workflow(definition.clone()) {
            Ok(output) => output.workflow,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        let analysis = merge_warnings(report, &compiled);

        let record = self
            .definitions
            .upsert(owner, definition, compiled, analysis, tags)
            .await;

        Ok(json!({
            "ok": true,
            "workflow_id": record.id,
            "version": record.version,
            "analysis": record.analysis,
            "definition": record.to_value(),
        }))
    }

    async fn get_definition(&self, params: &Value) -> RpcResult<Value> {
        let id = require_string(params, "workflow_id")?;
        match self.definitions.get_by_id(&id).await {
            Some(record) => Ok(json!({ "ok": true, "definition": record.to_value() })),
            None => Ok(not_found("workflow", &id)),
        }
    }

    async fn list_definitions(&self, params: &Value) -> RpcResult<Value> {
        let owner = optional_owner(params);
        let status = params
            .get("status")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_value::<DefinitionStatus>(json!(s)).ok());
        let tag = params
            .get("tag")
            .and_then(Value::as_str)
            .map(str::to_string);
        let records = self
            .definitions
            .list(owner.as_ref(), status, tag.as_deref())
            .await;
        Ok(json!({
            "ok": true,
            "definitions": records
                .iter()
                .map(|record| record.to_summary_value())
                .collect::<Vec<_>>(),
        }))
    }

    async fn archive_definition(&self, params: &Value) -> RpcResult<Value> {
        let id = require_string(params, "workflow_id")?;
        match self.definitions.archive(&id).await {
            Some(record) => Ok(json!({
                "ok": true,
                "workflow_id": record.id,
                "status": record.status,
            })),
            None => Ok(not_found("workflow", &id)),
        }
    }

    async fn dry_run(&self, params: &Value) -> RpcResult<Value> {
        let definition = require_definition(params)?;
        let (report, _ctx) = analyze_workflow(&definition);
        if report.has_errors() {
            return Ok(json!({
                "ok": false,
                "error": "analysis_failed",
                "analysis": report,
            }));
        }
        let compiled = match compile_workflow(definition) {
            Ok(output) => output.workflow,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        let merged = merge_warnings(report, &compiled);
        Ok(json!({
            "ok": true,
            "analysis": merged,
            "graph": compiled.graph,
        }))
    }

    // ----- §3.2 Workflow Run 生命周期 ------------------------------------

    async fn create_run(&self, params: &Value) -> RpcResult<Value> {
        let workflow_id = require_string(params, "workflow_id")?;
        let owner = require_owner(params)?;
        let trigger_input = params.get("input").cloned().unwrap_or(Value::Null);
        let callback_url = params
            .get("callback_url")
            .and_then(Value::as_str)
            .map(str::to_string);
        let auto_start = params
            .get("auto_start")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let definition = match self.definitions.get_by_id(&workflow_id).await {
            Some(record) => record,
            None => return Ok(not_found("workflow", &workflow_id)),
        };
        if definition.status == DefinitionStatus::Archived {
            return Ok(json!({
                "ok": false,
                "error": "definition_archived",
                "workflow_id": workflow_id,
            }));
        }

        let (mut run, mut events) = match self
            .orchestrator
            .create_run(&definition.compiled)
            .await
        {
            Ok(pair) => pair,
            Err(err) => return Ok(workflow_error_value(&err)),
        };

        // 把触发输入挂在 metrics["trigger_input"] 上，供后续节点的 Reference
        // 解析读取（按 §2.2 触发输入是 Run 的初始 metrics）。
        if !trigger_input.is_null() {
            run.metrics
                .insert("trigger_input".to_string(), trigger_input);
        }

        let mut record = RunRecord {
            run,
            workflow_id: definition.id.clone(),
            owner,
            events: Vec::new(),
            amendments: Vec::new(),
            callback_url,
        };

        if auto_start {
            // 等价 §3.2 文档里"create_run 后立刻调一次 start_run"的合法路径。
            match self
                .orchestrator
                .tick(&definition.compiled, &mut record.run)
                .await
            {
                Ok(more) => events.extend(more),
                Err(err) => return Ok(workflow_error_value(&err)),
            }
        }

        let run_id = record.run.run_id.clone();
        let status = record.run.status;
        let seq = record.run.seq;
        record.append_events(&events);
        let _ = self.runs.insert(record).await;
        Ok(json!({
            "ok": true,
            "run_id": run_id,
            "status": status,
            "events": events,
            "seq": seq,
        }))
    }

    async fn start_run(&self, params: &Value) -> RpcResult<Value> {
        let run_id = require_string(params, "run_id")?;
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
        let definition = match self.definitions.get_by_id(&handle.workflow_id).await {
            Some(d) => d,
            None => return Ok(not_found("workflow", &handle.workflow_id)),
        };
        let mut record = handle.state.lock().await;
        let pre_seq = record.run.seq;
        let events = match self
            .orchestrator
            .tick(&definition.compiled, &mut record.run)
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

    async fn tick_run(&self, params: &Value) -> RpcResult<Value> {
        // tick 与 start_run 在外部入口语义一致：都是 "从当前状态再推一次"。
        // 区别只是 start 一般是首次进入，文档把它们都放在 §3.2 的运维入口。
        self.start_run(params).await
    }

    async fn get_run_graph(&self, params: &Value) -> RpcResult<Value> {
        let run_id = require_string(params, "run_id")?;
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
        let definition = match self.definitions.get_by_id(&handle.workflow_id).await {
            Some(d) => d,
            None => return Ok(not_found("workflow", &handle.workflow_id)),
        };
        let record = handle.state.lock().await;
        Ok(json!({
            "ok": true,
            "run_id": record.run.run_id,
            "workflow_id": handle.workflow_id,
            "status": record.run.status,
            "graph": definition.compiled.graph,
            "nodes": definition.compiled.nodes,
            "node_states": record.run.node_states,
            "node_outputs": record.run.node_outputs,
            "human_waiting_nodes": record.run.human_waiting_nodes,
            "pending_thunks": record.run.pending_thunks,
            "metrics": record.run.metrics,
            "seq": record.run.seq,
        }))
    }

    async fn list_runs(&self, params: &Value) -> RpcResult<Value> {
        let owner = optional_owner(params);
        let workflow_id = params
            .get("workflow_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let status = params.get("status").and_then(Value::as_str).map(str::to_string);
        let handles = self
            .runs
            .list(owner.as_ref(), workflow_id.as_deref())
            .await;

        let mut out = Vec::with_capacity(handles.len());
        for handle in handles {
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
                "owner": handle.owner.to_value(),
                "created_at": record.run.created_at,
                "updated_at": record.run.updated_at,
                "seq": record.run.seq,
                "progress": record.run.progress_percent(),
            }));
        }
        Ok(json!({ "ok": true, "runs": out }))
    }

    // ----- §3.4 Agent / 外部系统集成 -------------------------------------

    async fn submit_step_output(&self, params: &Value) -> RpcResult<Value> {
        let run_id = require_string(params, "run_id")?;
        let node_id = require_string(params, "node_id")?;
        let actor = optional_actor(params);
        let output = params.get("output").cloned().unwrap_or(Value::Null);

        let (handle, definition) = match self.lookup_run(&run_id).await {
            Ok(pair) => pair,
            Err(payload) => return Ok(payload),
        };
        let mut record = handle.state.lock().await;
        let pre_seq = record.run.seq;
        let mut events = match self
            .orchestrator
            .submit_step_output(&definition.compiled, &mut record.run, &node_id, &actor, output)
            .await
        {
            Ok(events) => events,
            Err(err) => return Ok(workflow_error_value(&err)),
        };
        // 落完输出后再 tick 一次，让后继节点立即推进。
        match self
            .orchestrator
            .tick(&definition.compiled, &mut record.run)
            .await
        {
            Ok(more) => events.extend(more),
            Err(err) => return Ok(workflow_error_value(&err)),
        }
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

    async fn report_step_progress(&self, params: &Value) -> RpcResult<Value> {
        let run_id = require_string(params, "run_id")?;
        let node_id = require_string(params, "node_id")?;
        let actor = optional_actor(params);
        let progress = params.get("progress").cloned().unwrap_or(Value::Null);

        let (handle, definition) = match self.lookup_run(&run_id).await {
            Ok(pair) => pair,
            Err(payload) => return Ok(payload),
        };
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

    async fn request_human(&self, params: &Value) -> RpcResult<Value> {
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

    async fn submit_amendment(&self, params: &Value) -> RpcResult<Value> {
        let run_id = require_string(params, "run_id")?;
        let patch = params.get("patch").cloned().unwrap_or(Value::Null);
        let actor = optional_actor(params);
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
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

    async fn approve_amendment(&self, params: &Value) -> RpcResult<Value> {
        self.decide_amendment(params, AmendmentStatus::Approved).await
    }

    async fn reject_amendment(&self, params: &Value) -> RpcResult<Value> {
        self.decide_amendment(params, AmendmentStatus::Rejected).await
    }

    async fn decide_amendment(
        &self,
        params: &Value,
        status: AmendmentStatus,
    ) -> RpcResult<Value> {
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
        let mut record = handle.state.lock().await;
        let payload = {
            let amendment = match record
                .amendments
                .iter_mut()
                .find(|a| a.id == amendment_id)
            {
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
            // §3.4：通过审批后 plan_version + 1。真正按 patch 改写
            // CompiledWorkflow 的语义留给后续提交，这里先把版本号推进，让外部
            // 看得到状态机已经推进。
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

    async fn get_history(&self, params: &Value) -> RpcResult<Value> {
        let run_id = require_string(params, "run_id")?;
        let since_seq = params
            .get("since_seq")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(200) as usize;
        let handle = match self.runs.get(&run_id).await {
            Some(h) => h,
            None => return Ok(not_found("run", &run_id)),
        };
        let record = handle.state.lock().await;
        let events = record.events_since(since_seq, limit);
        let next_seq = events.last().map(|e| e.seq).unwrap_or(since_seq);
        Ok(json!({
            "ok": true,
            "run_id": run_id,
            "events": events,
            "next_seq": next_seq,
            "current_seq": record.run.seq,
        }))
    }

    async fn subscribe_events(&self, params: &Value) -> RpcResult<Value> {
        // §3.5：流式订阅经 kevent / kmsgqueue 投递。一期内 RPC 入口只
        // 给一个"指针"——告知订阅方走的 channel 名 + 当前 seq——具体的 push
        // 通道连接由调用方选择。同时返回最近 limit 条历史，便于断点续拉对齐。
        let run_id = require_string(params, "run_id")?;
        let history_payload = self.get_history(params).await?;
        Ok(json!({
            "ok": true,
            "channel": format!("workflow.events.{}", run_id),
            "transport": "kmsgqueue",
            "history": history_payload,
        }))
    }

    // ----- 共用辅助 -------------------------------------------------

    /// 拉 RunHandle + 对应 Definition，把 "run 不存在 / 引用的 Definition 不存在"
    /// 两种 not_found 路径折成一个 helper，避免每个 RPC 重复 6 行查表。
    async fn lookup_run(
        &self,
        run_id: &str,
    ) -> std::result::Result<(Arc<crate::state::RunHandle>, Arc<crate::state::DefinitionRecord>), Value>
    {
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

/// 把 `service.foo` 与裸 `foo` 都规整到同一个内部 case。
fn canonical_method(method: &str) -> &str {
    method.strip_prefix("service.").unwrap_or(method)
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

fn require_definition(params: &Value) -> RpcResult<WorkflowDefinition> {
    let raw = params
        .get("definition")
        .cloned()
        .ok_or_else(|| RPCErrors::ParseRequestError("missing required field `definition`".into()))?;
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

fn workflow_error_value(err: &WorkflowError) -> Value {
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
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::{ExecutorRegistry, InMemoryObjectStore, InMemoryThunkDispatcher};

    fn sample_definition_value() -> Value {
        // 一个最小可运行的 workflow：两步 + 一条边 + 一个人工节点。compile 通过即可。
        json!({
            "schema_version": "0.2.0",
            "id": "wf-test",
            "name": "test_workflow",
            "trigger": {"type": "manual"},
            "steps": [
                {
                    "id": "scan",
                    "name": "Scan",
                    "executor": "service::demo.scan",
                    "type": "autonomous",
                    "skippable": false,
                    "output_schema": {
                        "type": "object",
                        "properties": {"items": {"type": "array"}},
                        "required": ["items"]
                    }
                },
                {
                    "id": "approve",
                    "name": "Approve",
                    "type": "human_required",
                    "skippable": false,
                    "prompt": "ok?",
                    "output_schema": {
                        "type": "object",
                        "properties": {"decision": {"type": "string"}},
                        "required": ["decision"]
                    }
                }
            ],
            "edges": [
                {"from": "scan", "to": "approve"},
                {"from": "approve"}
            ]
        })
    }

    fn make_handler() -> WorkflowRpcHandler {
        let definitions = Arc::new(DefinitionStore::new());
        let runs = Arc::new(RunStore::new());
        let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
        let object_store = Arc::new(InMemoryObjectStore::new());
        let tracker = Arc::new(ServiceTracker::noop());
        let orchestrator = Arc::new(
            WorkflowOrchestrator::new(dispatcher, object_store, tracker)
                .with_executor_registry(Arc::new(ExecutorRegistry::new())),
        );
        WorkflowRpcHandler::new(definitions, runs, orchestrator)
    }

    fn make_req(method: &str, params: Value) -> RPCRequest {
        RPCRequest {
            method: method.to_string(),
            params,
            seq: 1,
            token: None,
            trace_id: None,
        }
    }

    #[tokio::test]
    async fn dispatch_unknown_method_returns_unknown() {
        let handler = make_handler();
        let err = handler
            .handle_rpc_call(make_req("nope", json!({})), "127.0.0.1".parse().unwrap())
            .await
            .expect_err("expected error");
        assert!(matches!(err, RPCErrors::UnknownMethod(_)));
    }

    #[tokio::test]
    async fn submit_then_get_definition_roundtrip() {
        let handler = make_handler();
        let resp = handler
            .handle_rpc_call(
                make_req(
                    "submit_definition",
                    json!({
                        "owner": {"user_id": "u", "app_id": "a"},
                        "definition": sample_definition_value(),
                    }),
                ),
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .expect("dispatch ok");
        let value = match resp.result {
            RPCResult::Success(v) => v,
            RPCResult::Failed(err) => panic!("submit failed: {:?}", err),
        };
        assert_eq!(value["ok"], true);
        let workflow_id = value["workflow_id"].as_str().unwrap().to_string();

        let resp = handler
            .handle_rpc_call(
                make_req("service.get_definition", json!({"workflow_id": workflow_id})),
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .expect("dispatch ok");
        let value = match resp.result {
            RPCResult::Success(v) => v,
            RPCResult::Failed(err) => panic!("get failed: {:?}", err),
        };
        assert_eq!(value["ok"], true);
        assert_eq!(value["definition"]["id"], json!(workflow_id));
    }

    #[tokio::test]
    async fn dry_run_returns_analysis_without_storing() {
        let handler = make_handler();
        let resp = handler
            .handle_rpc_call(
                make_req("dry_run", json!({"definition": sample_definition_value()})),
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .expect("dispatch ok");
        let value = match resp.result {
            RPCResult::Success(v) => v,
            RPCResult::Failed(err) => panic!("dry_run failed: {:?}", err),
        };
        assert_eq!(value["ok"], true);
        assert!(value["graph"].is_object());
    }

    #[tokio::test]
    async fn create_and_get_run_graph() {
        let handler = make_handler();
        let submit = handler
            .handle_rpc_call(
                make_req(
                    "submit_definition",
                    json!({
                        "owner": {"user_id": "u", "app_id": "a"},
                        "definition": sample_definition_value(),
                    }),
                ),
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        let workflow_id = match submit.result {
            RPCResult::Success(v) => v["workflow_id"].as_str().unwrap().to_string(),
            RPCResult::Failed(err) => panic!("submit failed: {:?}", err),
        };

        let create = handler
            .handle_rpc_call(
                make_req(
                    "create_run",
                    json!({
                        "workflow_id": workflow_id,
                        "owner": {"user_id": "u", "app_id": "a"},
                    }),
                ),
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        let run_id = match create.result {
            RPCResult::Success(v) => v["run_id"].as_str().unwrap().to_string(),
            RPCResult::Failed(err) => panic!("create failed: {:?}", err),
        };

        let graph = handler
            .handle_rpc_call(
                make_req("get_run_graph", json!({"run_id": run_id})),
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        let value = match graph.result {
            RPCResult::Success(v) => v,
            RPCResult::Failed(err) => panic!("graph failed: {:?}", err),
        };
        assert_eq!(value["ok"], true);
        assert!(value["nodes"].is_object());
        assert!(value["graph"].is_object());
    }
}
