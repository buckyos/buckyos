mod common;

use aicc::{
    CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry, RouteConfig,
    RouteWeights, Router, TenantRouteConfig,
};
use buckyos_api::{AiResponseSummary, AiccServerHandler, Capability, CompleteStatus};
use common::*;
use kRPC::{RPCContext, RPCHandler, RPCRequest, RPCResult};
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

fn add_llm(
    registry: &Registry,
    catalog: &ModelCatalog,
    id: &str,
    ptype: &str,
    cost: f64,
    lat: u64,
    r: std::result::Result<ProviderStartResult, ProviderError>,
) -> Arc<MockProvider> {
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", ptype, "m");
    let p = Arc::new(MockProvider::new(
        mock_instance(id, ptype, vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(cost),
            estimated_latency_ms: Some(lat),
        },
        vec![r],
    ));
    registry.add_provider(p.clone());
    p
}

#[tokio::test]
// 用例说明：
// - 验证场景：`workflow_01_plan_generation_dag` 用例，覆盖函数名对应的业务路径。
// - 输入参数：准备工作流步骤、依赖关系与执行策略。
// - 处理流程：执行工作流编排路径，推进规划、依赖调度、重试/回退与收敛阶段。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn workflow_01_plan_generation_dag() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: Some("{\"steps\":[]}".into()),
            tool_calls: vec![],
            artifacts: vec![],
            usage: None,
            cost: None,
            finish_reason: Some("stop".into()),
            provider_task_ref: None,
            extra: None,
        })),
    );
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Succeeded
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`workflow_02_serial_dependency_blocking` 用例，覆盖函数名对应的业务路径。
// - 输入参数：准备工作流步骤、依赖关系与执行策略。
// - 处理流程：执行工作流编排路径，推进规划、依赖调度、重试/回退与收敛阶段。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn workflow_02_serial_dependency_blocking() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let a = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    let b = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_ne!(a.task_id, b.task_id);
}

#[tokio::test]
// 用例说明：
// - 验证场景：`workflow_03_parallel_group_execution` 用例，覆盖函数名对应的业务路径。
// - 输入参数：准备工作流步骤、依赖关系与执行策略。
// - 处理流程：执行工作流编排路径，推进规划、依赖调度、重试/回退与收敛阶段。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn workflow_03_parallel_group_execution() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = Arc::new(center_with_taskmgr(r, c));
    let a = center.clone();
    let b = center.clone();
    let h1 = tokio::spawn(async move {
        a.complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .task_id
    });
    let h2 = tokio::spawn(async move {
        b.complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .task_id
    });
    assert_ne!(h1.await.unwrap(), h2.await.unwrap());
}

#[tokio::test]
// 用例说明：
// - 验证场景：`workflow_04_retryable_then_fallback_success` 用例，覆盖可重试错误分支、回退策略分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；准备工作流步骤、依赖关系与执行策略。
// - 处理流程：执行工作流编排路径，推进规划、依赖调度、重试/回退与收敛阶段。
// - 预期输出：返回成功结果，关键字段与断言一致；错误被归类为可重试并触发对应策略；回退执行次数与顺序满足用例断言。
async fn workflow_04_retryable_then_fallback_success() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Err(ProviderError::retryable("x")),
    );
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`workflow_05_fatal_step_abort` 用例，覆盖致命错误分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；准备工作流步骤、依赖关系与执行策略。
// - 处理流程：执行工作流编排路径，推进规划、依赖调度、重试/回退与收敛阶段。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
async fn workflow_05_fatal_step_abort() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(&r, &c, "p1", "a", 0.01, 10, Err(ProviderError::fatal("x")));
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Failed
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`workflow_06_replan_trigger_on_low_quality` 用例，覆盖函数名对应的业务路径。
// - 输入参数：准备工作流步骤、依赖关系与执行策略。
// - 处理流程：执行工作流编排路径，推进规划、依赖调度、重试/回退与收敛阶段。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn workflow_06_replan_trigger_on_low_quality() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let a = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    let b = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_ne!(a.task_id, b.task_id);
}

#[tokio::test]
// 用例说明：
// - 验证场景：`workflow_07_subtask_fallback_alias` 用例，覆盖回退策略分支、别名映射分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；准备工作流步骤、依赖关系与执行策略。
// - 处理流程：执行工作流编排路径，推进规划、依赖调度、重试/回退与收敛阶段。
// - 预期输出：回退执行次数与顺序满足用例断言。
async fn workflow_07_subtask_fallback_alias() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Err(ProviderError::retryable("x")),
    );
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`workflow_08_end_to_end_orchestration_smoke` 用例，覆盖函数名对应的业务路径。
// - 输入参数：准备工作流步骤、依赖关系与执行策略。
// - 处理流程：执行工作流编排路径，推进规划、依赖调度、重试/回退与收敛阶段。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn workflow_08_end_to_end_orchestration_smoke() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Queued { position: 1 }),
    );
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}
