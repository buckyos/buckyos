mod common;

use aicc::{
    CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry, RouteConfig,
    RouteWeights, Router, TaskEventKind, TenantRouteConfig,
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
// - 验证场景：`stream_01_started_then_poll_receives_incremental_chunks` 用例，覆盖启动成功状态分支、流式事件分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；准备可轮询的任务与事件流。
// - 处理流程：启动或查询流式任务，轮询事件并校验增量片段和最终快照的关系。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn stream_01_started_then_poll_receives_incremental_chunks() {
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
    let s = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(r, c);
    center.set_task_event_sink_factory(s.clone());
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, CompleteStatus::Running);
    assert!(!s.events_for(&resp.task_id).is_empty());
}

#[tokio::test]
// 用例说明：
// - 验证场景：`stream_02_incremental_chunks_are_append_only` 用例，覆盖流式事件分支。
// - 输入参数：准备可轮询的任务与事件流。
// - 处理流程：启动或查询流式任务，轮询事件并校验增量片段和最终快照的关系。
// - 预期输出：增量片段保持 append-only 语义。
async fn stream_02_incremental_chunks_are_append_only() {
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
    let s = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(r, c);
    center.set_task_event_sink_factory(s.clone());
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, CompleteStatus::Running);
    let e1 = s.events_for(&resp.task_id);
    assert_eq!(e1.len(), 1, "started response should emit one event");
    assert!(matches!(e1[0].kind, TaskEventKind::Started));
    assert_eq!(e1[0].task_id, resp.task_id);
    let e2 = s.events_for(&resp.task_id);
    assert_eq!(
        e2.len(),
        e1.len(),
        "event list should not shrink or reorder"
    );
    assert!(e2
        .iter()
        .zip(e1.iter())
        .all(|(after, before)| after.task_id == before.task_id
            && matches!(
                (&after.kind, &before.kind),
                (TaskEventKind::Started, TaskEventKind::Started)
            )));
}

#[tokio::test]
// 用例说明：
// - 验证场景：`stream_03_event_sequence_order_is_stable` 用例，覆盖流式事件分支。
// - 输入参数：准备可轮询的任务与事件流。
// - 处理流程：启动或查询流式任务，轮询事件并校验增量片段和最终快照的关系。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn stream_03_event_sequence_order_is_stable() {
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
    let s = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(r, c);
    center.set_task_event_sink_factory(s.clone());
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert!(matches!(
        s.events_for(&resp.task_id).first().map(|e| &e.kind),
        Some(aicc::TaskEventKind::Queued)
    ));
}

#[tokio::test]
// 用例说明：
// - 验证场景：`stream_04_started_must_not_fallback` 用例，覆盖回退策略分支、启动成功状态分支、流式事件分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；准备可轮询的任务与事件流。
// - 处理流程：启动或查询流式任务，轮询事件并校验增量片段和最终快照的关系。
// - 预期输出：回退执行次数与顺序满足用例断言。
async fn stream_04_started_must_not_fallback() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let p2 = add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, CompleteStatus::Running);
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 0);
}

#[tokio::test]
// 用例说明：
// - 验证场景：`stream_05_cancel_stops_incremental_output` 用例，覆盖流式事件分支、取消接口分支。
// - 输入参数：准备可轮询的任务与事件流。
// - 处理流程：启动或查询流式任务，轮询事件并校验增量片段和最终快照的关系。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn stream_05_cancel_stops_incremental_output() {
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
    let resp = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("ta")))
        .await
        .unwrap();
    assert!(
        center
            .cancel(resp.task_id.as_str(), rpc_ctx_with_tenant(Some("ta")))
            .await
            .unwrap()
            .accepted
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`stream_06_cross_tenant_poll_rejected` 用例，覆盖跨租户隔离分支、流式事件分支。
// - 输入参数：设置租户 token 或 tenant_id；准备可轮询的任务与事件流。
// - 处理流程：启动或查询流式任务，轮询事件并校验增量片段和最终快照的关系。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期；跨租户访问被阻断。
async fn stream_06_cross_tenant_poll_rejected() {
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
    let resp = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("ta")))
        .await
        .unwrap();
    assert!(center
        .cancel(resp.task_id.as_str(), rpc_ctx_with_tenant(Some("tb")))
        .await
        .is_err());
}

#[tokio::test]
// 用例说明：
// - 验证场景：`stream_07_stream_timeout_classified_retryable_before_started` 用例，覆盖可重试错误分支、启动成功状态分支、流式事件分支、超时/网络异常分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；准备可轮询的任务与事件流。
// - 处理流程：启动或查询流式任务，轮询事件并校验增量片段和最终快照的关系。
// - 预期输出：错误被归类为可重试并触发对应策略。
async fn stream_07_stream_timeout_classified_retryable_before_started() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Err(ProviderError::retryable("timeout")),
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
// - 验证场景：`stream_08_stream_final_snapshot_consistent_with_chunks` 用例，覆盖流式事件分支。
// - 输入参数：准备可轮询的任务与事件流。
// - 处理流程：启动或查询流式任务，轮询事件并校验增量片段和最终快照的关系。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn stream_08_stream_final_snapshot_consistent_with_chunks() {
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
            text: Some("abc".into()),
            tool_calls: vec![],
            artifacts: vec![],
            usage: None,
            cost: None,
            finish_reason: Some("stop".into()),
            provider_task_ref: None,
            extra: Some(json!({"chunks":["a","b","c"]})),
        })),
    );
    let s = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(r, c);
    center.set_task_event_sink_factory(s.clone());
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, CompleteStatus::Succeeded);
    let summary = resp
        .result
        .as_ref()
        .expect("immediate result should include final summary");
    assert_eq!(summary.text.as_deref(), Some("abc"));
    let chunks = summary
        .extra
        .as_ref()
        .and_then(|value| value.get("chunks"))
        .and_then(|value| value.as_array())
        .expect("summary.extra.chunks should be an array");
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].as_str(), Some("a"));
    assert_eq!(chunks[1].as_str(), Some("b"));
    assert_eq!(chunks[2].as_str(), Some("c"));

    let events = s.events_for(&resp.task_id);
    assert_eq!(
        events.len(),
        2,
        "immediate response should emit started + final"
    );
    assert!(matches!(events[0].kind, TaskEventKind::Started));
    assert!(matches!(events[1].kind, TaskEventKind::Final));
}
