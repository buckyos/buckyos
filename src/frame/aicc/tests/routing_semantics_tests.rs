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

#[test]
// 用例说明：
// - 验证场景：`route_01_mapped_primary_with_fallback` 用例，覆盖回退策略分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：回退执行次数与顺序满足用例断言。
fn route_01_mapped_primary_with_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p-a", "provider-a", "m-a", 0.01, 200);
    setup_route_provider(&registry, &catalog, "p-b", "provider-b", "m-b", 0.03, 300);

    let router = Router;
    let req = base_request();
    let snapshot = registry.snapshot(Capability::LlmRouter);
    let decision = router
        .route(
            "tenant-a",
            &req,
            &snapshot,
            &registry,
            &default_route_cfg(),
            &catalog,
        )
        .expect("route should succeed");

    assert_eq!(decision.primary_instance_id, "p-a", "assert_eq failed in route_01_mapped_primary_with_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(decision.fallback_instance_ids, vec!["p-b".to_string()], "assert_eq failed in route_01_mapped_primary_with_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[test]
// 用例说明：
// - 验证场景：`route_02_alias_unmapped_returns_model_alias_not_mapped` 用例，覆盖别名映射分支。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：返回 model_alias_not_mapped。
fn route_02_alias_unmapped_returns_model_alias_not_mapped() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let provider = Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::LlmRouter],
            vec!["plan".to_string()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(provider);

    let req = base_request();
    let snapshot = registry.snapshot(Capability::LlmRouter);
    let err = Router
        .route(
            "tenant-a",
            &req,
            &snapshot,
            &registry,
            &default_route_cfg(),
            &catalog,
        )
        .expect_err("route should fail");
    assert!(err.to_string().contains("model_alias_not_mapped"), "assert failed in route_02_alias_unmapped_returns_model_alias_not_mapped: condition is false; check preconditions and expected branch outcome.");
}

#[test]
// 用例说明：
// - 验证场景：`route_03_must_features_filtered_out` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn route_03_must_features_filtered_out() {
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
            vec!["json_output".to_string()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(provider);

    let req = base_request();
    let snapshot = registry.snapshot(Capability::LlmRouter);
    let err = Router
        .route(
            "tenant-a",
            &req,
            &snapshot,
            &registry,
            &default_route_cfg(),
            &catalog,
        )
        .expect_err("route should fail");
    assert!(err.to_string().contains("no_provider_available"), "assert failed in route_03_must_features_filtered_out: condition is false; check preconditions and expected branch outcome.");
}

#[test]
// 用例说明：
// - 验证场景：`route_04_tenant_allow_provider_types` 用例，覆盖租户 allow 供应方筛选分支。
// - 输入参数：设置租户 token 或 tenant_id；配置 tenant route override 的 allow/deny provider_types。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn route_04_tenant_allow_provider_types() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p-a", "provider-a", "m-a", 0.01, 100);
    setup_route_provider(&registry, &catalog, "p-b", "provider-b", "m-b", 0.005, 90);

    let mut cfg = default_route_cfg();
    cfg.tenant_overrides.insert(
        "tenant-a".to_string(),
        TenantRouteConfig {
            allow_provider_types: Some(vec!["provider-a".to_string()]),
            deny_provider_types: None,
            weights: None,
        },
    );
    let req = base_request();
    let snapshot = registry.snapshot(Capability::LlmRouter);
    let decision = Router
        .route("tenant-a", &req, &snapshot, &registry, &cfg, &catalog)
        .expect("route should succeed");
    assert_eq!(decision.primary_instance_id, "p-a", "assert_eq failed in route_04_tenant_allow_provider_types: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[test]
// 用例说明：
// - 验证场景：`route_05_tenant_deny_provider_types` 用例，覆盖租户 deny 供应方筛选分支。
// - 输入参数：设置租户 token 或 tenant_id；配置 tenant route override 的 allow/deny provider_types。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn route_05_tenant_deny_provider_types() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p-a", "provider-a", "m-a", 0.01, 100);
    setup_route_provider(&registry, &catalog, "p-b", "provider-b", "m-b", 0.005, 90);

    let mut cfg = default_route_cfg();
    cfg.tenant_overrides.insert(
        "tenant-a".to_string(),
        TenantRouteConfig {
            allow_provider_types: None,
            deny_provider_types: Some(vec!["provider-b".to_string()]),
            weights: None,
        },
    );
    let req = base_request();
    let snapshot = registry.snapshot(Capability::LlmRouter);
    let decision = Router
        .route("tenant-a", &req, &snapshot, &registry, &cfg, &catalog)
        .expect("route should succeed");
    assert_eq!(decision.primary_instance_id, "p-a", "assert_eq failed in route_05_tenant_deny_provider_types: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[test]
// 用例说明：
// - 验证场景：`route_06_max_cost_filter` 用例，覆盖成本阈值过滤分支。
// - 输入参数：设置 max_cost_usd 与不同成本候选。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn route_06_max_cost_filter() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p-a", "provider-a", "m-a", 0.5, 100);

    let mut req = base_request();
    req.requirements.max_cost_usd = Some(0.01);
    let snapshot = registry.snapshot(Capability::LlmRouter);
    let err = Router
        .route(
            "tenant-a",
            &req,
            &snapshot,
            &registry,
            &default_route_cfg(),
            &catalog,
        )
        .expect_err("route should fail by cost");
    assert!(err.to_string().contains("no_provider_available"), "assert failed in route_06_max_cost_filter: condition is false; check preconditions and expected branch outcome.");
}

#[test]
// 用例说明：
// - 验证场景：`route_07_max_latency_filter` 用例，覆盖延迟阈值过滤分支。
// - 输入参数：设置 max_latency_ms 与不同延迟候选。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn route_07_max_latency_filter() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p-a", "provider-a", "m-a", 0.001, 9000);

    let mut req = base_request();
    req.requirements.max_latency_ms = Some(500);
    let snapshot = registry.snapshot(Capability::LlmRouter);
    let err = Router
        .route(
            "tenant-a",
            &req,
            &snapshot,
            &registry,
            &default_route_cfg(),
            &catalog,
        )
        .expect_err("route should fail by latency");
    assert!(err.to_string().contains("no_provider_available"), "assert failed in route_07_max_latency_filter: condition is false; check preconditions and expected branch outcome.");
}

#[test]
// 用例说明：
// - 验证场景：`route_08_tenant_mapping_override_global` 用例，覆盖函数名对应的业务路径。
// - 输入参数：设置租户 token 或 tenant_id。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn route_08_tenant_mapping_override_global() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(
        &registry,
        &catalog,
        "p-a",
        "provider-a",
        "m-global",
        0.01,
        100,
    );
    catalog.set_tenant_mapping(
        "tenant-a",
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-tenant",
    );

    let req = base_request();
    let snapshot = registry.snapshot(Capability::LlmRouter);
    let decision = Router
        .route(
            "tenant-a",
            &req,
            &snapshot,
            &registry,
            &default_route_cfg(),
            &catalog,
        )
        .expect("route should succeed");
    assert_eq!(decision.provider_model, "m-tenant", "assert_eq failed in route_08_tenant_mapping_override_global: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`route_08_tenant_mapping_override_global_on_complete` 用例，覆盖函数名对应的业务路径。
// - 输入参数：设置租户 token 或 tenant_id。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn route_08_tenant_mapping_override_global_on_complete() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "global-model",
    );
    catalog.set_tenant_mapping(
        "tenant-x",
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "tenant-model",
    );
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::LlmRouter],
            vec!["plan".to_string()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let center = center_with_taskmgr(registry, catalog);
    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-x")))
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
    assert_eq!(
        task.data
            .pointer("/aicc/route/provider_model")
            .and_then(|v| v.as_str()),
        Some("tenant-model")
    , "assert_eq failed in route_08_tenant_mapping_override_global_on_complete: expected left == right; check this scenario's routing/status/error-code branch.");
}
