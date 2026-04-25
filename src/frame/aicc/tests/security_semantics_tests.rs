mod common;

use aicc::{
    AIComputeCenter, CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry,
    Router, TaskEventKind, TenantRouteConfig,
};
use buckyos_api::{
    AiMethodStatus, AiResponseSummary, Capability, ResourceRef, TaskFilter, TaskStatus,
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
    catalog.set_mapping(Capability::Llm, "llm.plan.default", provider_type, model);
    let provider = Arc::new(MockProvider::new(
        mock_instance(
            instance_id,
            provider_type,
            vec![Capability::Llm],
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
// - 验证场景：`sec_01_cancel_reject_cross_tenant` 用例，覆盖跨租户隔离分支、取消接口分支。
// - 输入参数：设置租户 token 或 tenant_id。
// - 处理流程：调用安全相关接口，触发租户边界、资源有效性和策略校验。
// - 预期输出：跨租户访问被阻断。
async fn sec_01_cancel_reject_cross_tenant() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::Llm],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let center = center_with_taskmgr(registry, catalog);

    let start = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-alice")))
        .await
        .unwrap();
    let err = center
        .cancel(
            start.task_id.as_str(),
            rpc_ctx_with_tenant(Some("tenant-bob")),
        )
        .await
        .expect_err("cross tenant cancel must fail");
    assert!(err.to_string().contains("NoPermission") || err.to_string().contains("cross-tenant"), "assert failed in sec_01_cancel_reject_cross_tenant: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`sec_02_cancel_accept_same_tenant` 用例，覆盖同租户授权分支、取消接口分支。
// - 输入参数：设置租户 token 或 tenant_id。
// - 处理流程：调用安全相关接口，触发租户边界、资源有效性和策略校验。
// - 预期输出：同租户请求可执行并返回可预期结果。
async fn sec_02_cancel_accept_same_tenant() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::Llm],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let center = center_with_taskmgr(registry, catalog);

    let start = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-alice")))
        .await
        .unwrap();
    let resp = center
        .cancel(
            start.task_id.as_str(),
            rpc_ctx_with_tenant(Some("tenant-alice")),
        )
        .await
        .unwrap();
    assert!(resp.accepted, "assert failed in sec_02_cancel_accept_same_tenant: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`sec_03_resource_invalid_from_resolver` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：调用安全相关接口，触发租户边界、资源有效性和策略校验。
// - 预期输出：返回成功结果，关键字段与断言一致；返回拒绝或致命错误，错误码/错误消息符合预期。
async fn sec_03_resource_invalid_from_resolver() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    center.set_resource_resolver(Arc::new(FailingResolver {
        message: "resolver denied".to_string(),
    }));
    center.registry().add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::Llm],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    center
        .model_catalog()
        .set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(response.status, AiMethodStatus::Failed, "assert_eq failed in sec_03_resource_invalid_from_resolver: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in sec_03_resource_invalid_from_resolver: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`sec_04_base64_policy_enforced` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：调用安全相关接口，触发租户边界、资源有效性和策略校验。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn sec_04_base64_policy_enforced() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    center.set_base64_policy(8, string_set(&["image/png"]));

    let req = request_with_resource(ResourceRef::Base64 {
        mime: "audio/wav".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    });
    let response = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(response.status, AiMethodStatus::Failed, "assert_eq failed in sec_04_base64_policy_enforced: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in sec_04_base64_policy_enforced: expected left == right; check this scenario's routing/status/error-code branch.");
}
