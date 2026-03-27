mod common;

use aicc::{
    AIComputeCenter, CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry,
    Router, TaskEventKind, TenantRouteConfig,
};
use buckyos_api::{
    AiResponseSummary, Capability, CompleteStatus, ResourceRef, TaskFilter, TaskStatus,
};
use common::*;
use std::collections::HashMap;
use std::sync::Arc;

fn setup_route_provider(
    registry: &Registry,
    catalog: &ModelCatalog,
    instance_id: &str,
    provider_type: &str,
    model: &str,
    cost: f64,
    latency_ms: u64,
) {
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        provider_type,
        model,
    );
    let provider = Arc::new(MockProvider::new(
        mock_instance(
            instance_id,
            provider_type,
            vec![Capability::LlmRouter],
            vec!["plan".to_string()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(cost),
            estimated_latency_ms: Some(latency_ms),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(provider);
}

#[tokio::test]
// 用例说明：
// - 验证场景：`task_01_immediate_persists_completed` 用例，覆盖函数名对应的业务路径。
// - 输入参数：准备任务状态持久化与事件存储上下文。
// - 处理流程：执行任务生命周期推进，在关键状态节点落盘并记录事件。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn task_01_immediate_persists_completed() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: Some("ok".to_string()),
            ..Default::default()
        }))],
    )));
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(response.status, CompleteStatus::Succeeded, "assert_eq failed in task_01_immediate_persists_completed: expected left == right; check this scenario's routing/status/error-code branch.");

    let taskmgr = center.task_manager_client().expect("task manager");
    let tasks = taskmgr
        .list_tasks(None::<TaskFilter>, None, None)
        .await
        .expect("list tasks");
    let task = tasks
        .into_iter()
        .find(|t| {
            t.data
                .pointer("/aicc/external_task_id")
                .and_then(|v| v.as_str())
                == Some(response.task_id.as_str())
        })
        .expect("task should exist");
    assert_eq!(task.status, TaskStatus::Completed, "assert_eq failed in task_01_immediate_persists_completed: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`task_02_started_persists_running_and_binding` 用例，覆盖启动成功状态分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；准备任务状态持久化与事件存储上下文。
// - 处理流程：执行任务生命周期推进，在关键状态节点落盘并记录事件。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn task_02_started_persists_running_and_binding() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    let provider = Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(provider.clone());
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .unwrap();
    assert_eq!(response.status, CompleteStatus::Running, "assert_eq failed in task_02_started_persists_running_and_binding: expected left == right; check this scenario's routing/status/error-code branch.");
    let cancel = center
        .cancel(
            response.task_id.as_str(),
            rpc_ctx_with_tenant(Some("tenant-a")),
        )
        .await
        .unwrap();
    assert!(cancel.accepted, "assert failed in task_02_started_persists_running_and_binding: condition is false; check preconditions and expected branch outcome.");
    assert_eq!(provider.canceled_tasks(), vec![response.task_id], "assert_eq failed in task_02_started_persists_running_and_binding: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`task_03_queued_persists_pending_and_position` 用例，覆盖排队状态分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；准备任务状态持久化与事件存储上下文。
// - 处理流程：执行任务生命周期推进，在关键状态节点落盘并记录事件。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn task_03_queued_persists_pending_and_position() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Queued { position: 7 })],
    )));
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let taskmgr = center.task_manager_client().expect("task manager");
    let tasks = taskmgr
        .list_tasks(None::<TaskFilter>, None, None)
        .await
        .expect("list tasks");
    let task = tasks
        .into_iter()
        .find(|t| {
            t.data
                .pointer("/aicc/external_task_id")
                .and_then(|v| v.as_str())
                == Some(response.task_id.as_str())
        })
        .expect("task should exist");
    assert_eq!(task.status, TaskStatus::Pending, "assert_eq failed in task_03_queued_persists_pending_and_position: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(
        task.data.pointer("/aicc/events/0/kind").and_then(|v| v.as_str()),
        Some("queued")
    , "assert_eq failed in task_03_queued_persists_pending_and_position: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`task_04_emit_error_event_with_code` 用例，覆盖函数名对应的业务路径。
// - 输入参数：准备任务状态持久化与事件存储上下文。
// - 处理流程：执行任务生命周期推进，在关键状态节点落盘并记录事件。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn task_04_emit_error_event_with_code() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());

    let mut req = base_request();
    req.model.alias = "   ".to_string();
    let response = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(response.status, CompleteStatus::Failed, "assert_eq failed in task_04_emit_error_event_with_code: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("bad_request"), "assert_eq failed in task_04_emit_error_event_with_code: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`task_04_emit_error_event_has_error_kind` 用例，覆盖函数名对应的业务路径。
// - 输入参数：准备任务状态持久化与事件存储上下文。
// - 处理流程：执行任务生命周期推进，在关键状态节点落盘并记录事件。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn task_04_emit_error_event_has_error_kind() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let mut req = base_request();
    req.model.alias = "".to_string();
    let response = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let events = sink.events_for(&response.task_id);
    assert!(events.iter().any(|e| matches!(e.kind, TaskEventKind::Error)), "assert failed in task_04_emit_error_event_has_error_kind: condition is false; check preconditions and expected branch outcome.");
}
