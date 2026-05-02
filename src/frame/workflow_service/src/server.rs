//! kRPC 入口分发。
//!
//! 方法清单与 [doc/workflow/workflow service.md](../../doc/workflow/workflow%20service.md) §3
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
//!
//! 现阶段所有方法仅返回结构化的 `not_implemented` 占位，把 method dispatch 通路
//! 跑通。后续提交会把每个 stub 替换为真正的实现：解码请求 → 调 workflow crate 的
//! orchestrator/object store/task tracker → 编码响应。

use ::kRPC::*;
use serde_json::{json, Value};
use std::net::IpAddr;

type RpcResult<T> = std::result::Result<T, RPCErrors>;

/// 把 method dispatch 集中起来；后续每个 stub 都会替换为真正的 handler。
pub struct WorkflowRpcHandler {
    // 后续接入：
    //   orchestrator: Arc<WorkflowOrchestrator<...>>,
    //   definitions:  Arc<dyn DefinitionStore>,
    //   runs:         Arc<dyn RunStore>,
    //   events:       Arc<dyn EventLog>,
    //   subscriptions: Arc<EventBus>,
    // 现在留空，避免在没有持久化层之前就锁死接口。
}

impl WorkflowRpcHandler {
    pub fn new() -> Self {
        Self {}
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

    async fn submit_definition(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("submit_definition")
    }

    async fn get_definition(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("get_definition")
    }

    async fn list_definitions(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("list_definitions")
    }

    async fn archive_definition(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("archive_definition")
    }

    async fn dry_run(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("dry_run")
    }

    // ----- §3.2 Workflow Run 生命周期 ------------------------------------

    async fn create_run(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("create_run")
    }

    async fn start_run(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("start_run")
    }

    async fn tick_run(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("tick_run")
    }

    async fn get_run_graph(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("get_run_graph")
    }

    async fn list_runs(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("list_runs")
    }

    // ----- §3.4 Agent / 外部系统集成 -------------------------------------

    async fn submit_step_output(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("submit_step_output")
    }

    async fn report_step_progress(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("report_step_progress")
    }

    async fn request_human(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("request_human")
    }

    // ----- §3.4 Amendment ------------------------------------------------

    async fn submit_amendment(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("submit_amendment")
    }

    async fn approve_amendment(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("approve_amendment")
    }

    async fn reject_amendment(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("reject_amendment")
    }

    // ----- §3.5 事件订阅 -------------------------------------------------

    async fn get_history(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("get_history")
    }

    async fn subscribe_events(&self, _params: &Value) -> RpcResult<Value> {
        not_implemented("subscribe_events")
    }
}

/// 把 `service.foo` 与裸 `foo` 都规整到同一个内部 case。
fn canonical_method(method: &str) -> &str {
    method.strip_prefix("service.").unwrap_or(method)
}

fn not_implemented(method: &str) -> RpcResult<Value> {
    log::warn!("workflow.{} not implemented yet", method);
    Ok(json!({
        "ok": false,
        "error": "not_implemented",
        "method": method,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req(method: &str) -> RPCRequest {
        RPCRequest {
            method: method.to_string(),
            params: json!({}),
            seq: 1,
            token: None,
            trace_id: None,
        }
    }

    #[tokio::test]
    async fn dispatch_known_method_returns_not_implemented_payload() {
        let handler = WorkflowRpcHandler::new();
        let resp = handler
            .handle_rpc_call(make_req("submit_definition"), "127.0.0.1".parse().unwrap())
            .await
            .expect("dispatch ok");
        match resp.result {
            RPCResult::Success(value) => {
                assert_eq!(value["error"], json!("not_implemented"));
                assert_eq!(value["method"], json!("submit_definition"));
            }
            RPCResult::Failed(err) => panic!("unexpected failure: {:?}", err),
        }
    }

    #[tokio::test]
    async fn dispatch_accepts_service_prefix() {
        let handler = WorkflowRpcHandler::new();
        let resp = handler
            .handle_rpc_call(make_req("service.create_run"), "127.0.0.1".parse().unwrap())
            .await
            .expect("dispatch ok");
        assert!(matches!(resp.result, RPCResult::Success(_)));
    }

    #[tokio::test]
    async fn dispatch_unknown_method_returns_unknown() {
        let handler = WorkflowRpcHandler::new();
        let err = handler
            .handle_rpc_call(make_req("nope"), "127.0.0.1".parse().unwrap())
            .await
            .expect_err("expected error");
        assert!(matches!(err, RPCErrors::UnknownMethod(_)));
    }

}
