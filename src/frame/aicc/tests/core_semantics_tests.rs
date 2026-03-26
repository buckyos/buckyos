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
// 意图：验证路由选择与主备排序场景（route_01_mapped_primary_with_fallback）。预期返回断言中的 primary/fallback 结果，因为该输入会命中对应业务分支；不应返回与排序策略相反的结果，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证模型别名未映射场景（route_02_alias_unmapped_returns_model_alias_not_mapped）。预期返回 model_alias_not_mapped，因为该输入会命中对应业务分支；不应返回 no_provider_available，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证must_features 过滤场景（route_03_must_features_filtered_out）。预期返回 no_provider_available，因为该输入会命中对应业务分支；不应返回成功，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证租户白名单限制场景（route_04_tenant_allow_provider_types）。预期只命中白名单 provider，因为该输入会命中对应业务分支；不应命中其他 provider，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证租户黑名单限制场景（route_05_tenant_deny_provider_types）。预期避开被拒绝的 provider，因为该输入会命中对应业务分支；不应选中黑名单 provider，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证成本上限过滤场景（route_06_max_cost_filter）。预期返回 no_provider_available，因为该输入会命中对应业务分支；不应放行超限候选，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证时延上限过滤场景（route_07_max_latency_filter）。预期返回 no_provider_available，因为该输入会命中对应业务分支；不应放行超时延候选，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证租户映射覆盖全局场景（route_08_tenant_mapping_override_global）。预期使用 tenant 映射模型，因为该输入会命中对应业务分支；不应回退到 global 映射，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证启动可重试失败回退场景（start_01_retryable_error_then_fallback_success）。预期回退后 Running 或成功启动，因为该输入会命中对应业务分支；不应立即 Failed，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn start_01_retryable_error_then_fallback_success() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-b",
        "m-b",
    );
    let p1 = Arc::new(MockProvider::new(
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
        vec![Err(ProviderError::retryable("temp"))],
    ));
    let p2 = Arc::new(MockProvider::new(
        mock_instance(
            "p-b",
            "provider-b",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.02),
            estimated_latency_ms: Some(200),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(p1.clone());
    registry.add_provider(p2.clone());
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .expect("complete should return");
    assert_eq!(response.status, CompleteStatus::Running, "assert_eq failed in start_01_retryable_error_then_fallback_success: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(p1.start_calls(), 1, "assert_eq failed in start_01_retryable_error_then_fallback_success: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(p2.start_calls(), 1, "assert_eq failed in start_01_retryable_error_then_fallback_success: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证启动致命错误短路场景（start_02_fatal_error_no_fallback）。预期立即 Failed 且不回退，因为该输入会命中对应业务分支；不应继续尝试其它候选，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn start_02_fatal_error_no_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-b",
        "m-b",
    );
    let p1 = Arc::new(MockProvider::new(
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
        vec![Err(ProviderError::fatal("bad request"))],
    ));
    let p2 = Arc::new(MockProvider::new(
        mock_instance(
            "p-b",
            "provider-b",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.02),
            estimated_latency_ms: Some(200),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(p1.clone());
    registry.add_provider(p2.clone());

    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .expect("complete should return");
    assert_eq!(response.status, CompleteStatus::Failed, "assert_eq failed in start_02_fatal_error_no_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(p1.start_calls(), 1, "assert_eq failed in start_02_fatal_error_no_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(p2.start_calls(), 0, "assert_eq failed in start_02_fatal_error_no_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证启动状态机场景（start_03_started_must_stop_fallback）。预期返回断言中状态，因为该输入会命中对应业务分支；不应返回与状态机不符的状态，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn start_03_started_must_stop_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-b",
        "m-b",
    );
    let p1 = Arc::new(MockProvider::new(
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
    let p2 = Arc::new(MockProvider::new(
        mock_instance(
            "p-b",
            "provider-b",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.02),
            estimated_latency_ms: Some(200),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(p1.clone());
    registry.add_provider(p2.clone());
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .expect("complete should return");
    assert_eq!(response.status, CompleteStatus::Running, "assert_eq failed in start_03_started_must_stop_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(p1.start_calls(), 1, "assert_eq failed in start_03_started_must_stop_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(p2.start_calls(), 0, "assert_eq failed in start_03_started_must_stop_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证启动 Queued 语义场景（start_04_queued_no_fallback）。预期保持 Running 且不重复启动，因为该输入会命中对应业务分支；不应立即转 Failed 或再次回退，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn start_04_queued_no_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-b",
        "m-b",
    );
    let p1 = Arc::new(MockProvider::new(
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
        vec![Ok(ProviderStartResult::Queued { position: 2 })],
    ));
    let p2 = Arc::new(MockProvider::new(
        mock_instance(
            "p-b",
            "provider-b",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.02),
            estimated_latency_ms: Some(200),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(p1.clone());
    registry.add_provider(p2.clone());
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .expect("complete should return");
    assert_eq!(response.status, CompleteStatus::Running, "assert_eq failed in start_04_queued_no_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(p1.start_calls(), 1, "assert_eq failed in start_04_queued_no_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(p2.start_calls(), 0, "assert_eq failed in start_04_queued_no_fallback: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证全候选启动失败聚合场景（start_05_all_candidates_failed_provider_start_failed）。预期错误码 provider_start_failed，因为该输入会命中对应业务分支；不应错分为 no_provider_available，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn start_05_all_candidates_failed_provider_start_failed() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-b",
        "m-b",
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
        vec![Err(ProviderError::retryable("retry-a"))],
    )));
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-b",
            "provider-b",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.02),
            estimated_latency_ms: Some(200),
        },
        vec![Err(ProviderError::retryable("retry-b"))],
    )));

    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .expect("complete should return");
    assert_eq!(response.status, CompleteStatus::Failed, "assert_eq failed in start_05_all_candidates_failed_provider_start_failed: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("provider_start_failed"), "assert_eq failed in start_05_all_candidates_failed_provider_start_failed: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证回退次数上限场景（start_06_fallback_respects_limit）。预期在 fallback_limit 内结束尝试，因为该输入会命中对应业务分支；不应无限制重试，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn start_06_fallback_respects_limit() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    for (id, ptype, model, cost) in [
        ("p-a", "provider-a", "m-a", 0.01),
        ("p-b", "provider-b", "m-b", 0.02),
        ("p-c", "provider-c", "m-c", 0.03),
        ("p-d", "provider-d", "m-d", 0.04),
    ] {
        catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", ptype, model);
        registry.add_provider(Arc::new(MockProvider::new(
            mock_instance(id, ptype, vec![Capability::LlmRouter], vec!["plan".into()]),
            CostEstimate {
                estimated_cost_usd: Some(cost),
                estimated_latency_ms: Some(100),
            },
            vec![Err(ProviderError::retryable("retry"))],
        )));
    }

    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    center.update_route_config(aicc::RouteConfig {
        fallback_limit: 2,
        ..default_route_cfg()
    });

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .expect("complete should return");
    assert_eq!(response.status, CompleteStatus::Failed, "assert_eq failed in start_06_fallback_respects_limit: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("provider_start_failed"), "assert_eq failed in start_06_fallback_respects_limit: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证任务立即完成持久化场景（task_01_immediate_persists_completed）。预期落库 Completed，因为该输入会命中对应业务分支；不应保持 Pending/Running，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证任务 Started 绑定与取消场景（task_02_started_persists_running_and_binding）。预期保持 Running 且 cancel 命中原 provider，因为该输入会命中对应业务分支；不应取消到错误实例或被拒绝，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证任务 Queued 持久化场景（task_03_queued_persists_pending_and_position）。预期落库 Pending 并保留 position，因为该输入会命中对应业务分支；不应误标为 Completed，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证错误事件内容场景（task_04_emit_error_event_with_code）。预期事件包含 code，因为该输入会命中对应业务分支；不应只有文本描述无机读码，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证跨租户取消授权场景（sec_01_cancel_reject_cross_tenant）。预期被拒绝，因为该输入会命中对应业务分支；不应允许越权取消，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn sec_01_cancel_reject_cross_tenant() {
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
// 意图：验证同租户取消授权场景（sec_02_cancel_accept_same_tenant）。预期被接受，因为该输入会命中对应业务分支；不应误拒绝合法操作，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn sec_02_cancel_accept_same_tenant() {
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
// 意图：验证资源解析错误归一化场景（sec_03_resource_invalid_from_resolver）。预期错误码 resource_invalid，因为该输入会命中对应业务分支；不应泄露内部解析错误形态，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn sec_03_resource_invalid_from_resolver() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
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
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    center.model_catalog().set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );

    let response = center
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(response.status, CompleteStatus::Failed, "assert_eq failed in sec_03_resource_invalid_from_resolver: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in sec_03_resource_invalid_from_resolver: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证Base64 策略校验场景（sec_04_base64_policy_enforced）。预期不合规输入返回 resource_invalid，因为该输入会命中对应业务分支；不应放行违规数据，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert_eq!(response.status, CompleteStatus::Failed, "assert_eq failed in sec_04_base64_policy_enforced: expected left == right; check this scenario's routing/status/error-code branch.");
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in sec_04_base64_policy_enforced: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证错误码映射一致性场景（obs_01_error_code_mapping_consistent）。预期同类语义稳定映射固定 code，因为该输入会命中对应业务分支；不应随路径漂移，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn obs_01_error_code_mapping_consistent() {
    let sink = Arc::new(CollectingSinkFactory::new());

    let center_bad = {
        let mut c = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
        c.set_task_event_sink_factory(sink.clone());
        c
    };
    let mut bad_req = base_request();
    bad_req.model.alias = "".to_string();
    let bad_resp = center_bad
        .complete(bad_req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&bad_resp.task_id)).as_deref(), Some("bad_request"), "assert_eq failed in obs_01_error_code_mapping_consistent: expected left == right; check this scenario's routing/status/error-code branch.");

    let center_no_provider = {
        let mut c = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
        c.set_task_event_sink_factory(sink.clone());
        c
    };
    let no_provider_resp = center_no_provider
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&no_provider_resp.task_id)).as_deref(), Some("no_provider_available"), "assert_eq failed in obs_01_error_code_mapping_consistent: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证日志脱敏场景（obs_02_log_redaction_no_prompt_or_base64）。预期不包含敏感 prompt/base64 原文，因为该输入会命中对应业务分支；不应泄露私密内容，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn obs_02_log_redaction_no_prompt_or_base64() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let secret = "very-sensitive-base64";
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: secret.to_string(),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let events = sink.events_for(&resp.task_id);
    let msg = events
        .iter()
        .filter_map(|e| e.data.as_ref())
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!msg.contains(secret), "assert failed in obs_02_log_redaction_no_prompt_or_base64: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证并发 task_id 唯一性场景（conc_01_task_id_uniqueness_under_concurrency）。预期task_id 全部唯一，因为该输入会命中对应业务分支；不应出现重复冲突，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn conc_01_task_id_uniqueness_under_concurrency() {
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
        vec![
            Ok(ProviderStartResult::Immediate(AiResponseSummary {
                text: Some("ok".to_string()),
                ..Default::default()
            }));
            128
        ],
    )));
    let center = Arc::new(center_with_taskmgr(registry, catalog));

    let mut handles = vec![];
    for _ in 0..64 {
        let c = center.clone();
        handles.push(tokio::spawn(async move {
            c.complete(base_request(), rpc_ctx_with_tenant(None))
                .await
                .expect("complete")
                .task_id
        }));
    }
    let mut ids = Vec::with_capacity(handles.len());
    for h in handles {
        ids.push(h.await.expect("join"));
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 64, "assert_eq failed in conc_01_task_id_uniqueness_under_concurrency: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证注册表热更新一致性场景（conc_02_registry_hot_update_route_consistency）。预期路由持续可用，因为该输入会命中对应业务分支；不应出现随机路由失败，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn conc_02_registry_hot_update_route_consistency() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "m-a",
    );
    let router = Router;
    let req = base_request();

    for i in 0..40usize {
        let id = format!("hot-{i}");
        registry.add_provider(Arc::new(MockProvider::new(
            mock_instance(
                id.as_str(),
                "provider-a",
                vec![Capability::LlmRouter],
                vec!["plan".into()],
            ),
            CostEstimate {
                estimated_cost_usd: Some(0.01),
                estimated_latency_ms: Some(100),
            },
            vec![Ok(ProviderStartResult::Started)],
        )));

        let snapshot = registry.snapshot(Capability::LlmRouter);
        let result = router.route(
            "tenant-a",
            &req,
            &snapshot,
            &registry,
            &default_route_cfg(),
            &catalog,
        );
        assert!(result.is_ok(), "route should stay stable during updates");
        registry.remove_instance(id.as_str());
    }
}

#[tokio::test]
// 意图：验证Base64 MIME 白名单场景（proto_b64_06_invalid_mime_rejected）。预期返回 resource_invalid，因为该输入会命中对应业务分支；不应放行非法 MIME，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_b64_06_invalid_mime_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/gif".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in proto_b64_06_invalid_mime_rejected: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证Base64 大小上限场景（proto_b64_07_size_limit_exceeded_rejected）。预期返回 resource_invalid，因为该输入会命中对应业务分支；不应绕过大小限制，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_b64_07_size_limit_exceeded_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    center.set_base64_policy(2, string_set(&["image/png"]));
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(&[1, 2, 3, 4]),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in proto_b64_07_size_limit_exceeded_rejected: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证Base64 编码合法性场景（proto_b64_08_malformed_base64_rejected）。预期返回 resource_invalid，因为该输入会命中对应业务分支；不应容错解码非法输入，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_b64_08_malformed_base64_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: "%%%not-base64%%%".to_string(),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in proto_b64_08_malformed_base64_rejected: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证URL 格式校验场景（proto_url_03_missing_scheme_rejected）。预期返回 resource_invalid，因为该输入会命中对应业务分支；不应隐式补全 scheme，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_url_03_missing_scheme_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Url {
        url: "example.com/image.png".to_string(),
        mime_hint: None,
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in proto_url_03_missing_scheme_rejected: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证URL 空值校验场景（proto_url_04_empty_url_rejected）。预期返回 resource_invalid，因为该输入会命中对应业务分支；不应继续解析空输入，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_url_04_empty_url_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Url {
        url: " ".to_string(),
        mime_hint: None,
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in proto_url_04_empty_url_rejected: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证协议安全字段透传场景（proto_sec_04_idempotency_key_preserved）。预期关键字段保持不变，因为该输入会命中对应业务分支；不应丢失或篡改请求字段，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_sec_04_idempotency_key_preserved() {
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
        vec![Ok(ProviderStartResult::Started)],
    )));
    let center = center_with_taskmgr(registry, catalog);
    let req = base_request();
    let idem = req.idempotency_key.clone().expect("idem key");
    let response = center
        .complete(req, rpc_ctx_with_tenant(None))
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
            .pointer("/aicc/request/idempotency_key")
            .and_then(|v| v.as_str()),
        Some(idem.as_str())
    , "assert_eq failed in proto_sec_04_idempotency_key_preserved: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证URL 不可达处理场景（proto_url_06_resource_unreachable_simulated）。预期返回 resource_invalid，因为该输入会命中对应业务分支；不应误分类为 provider_start_failed，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_url_06_resource_unreachable_simulated() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    center.set_resource_resolver(Arc::new(FailingResolver {
        message: "resource unreachable".to_string(),
    }));
    let req = request_with_resource(ResourceRef::Url {
        url: "https://example.com/1.png".to_string(),
        mime_hint: None,
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_eq failed in proto_url_06_resource_unreachable_simulated: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证启动失败错误码场景（obs_01_provider_start_failed_code）。预期记录 provider_start_failed，因为该输入会命中对应业务分支；不应映射到无关 code，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn obs_01_provider_start_failed_code() {
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
        vec![Err(ProviderError::fatal("boom"))],
    )));
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("provider_start_failed"), "assert_eq failed in obs_01_provider_start_failed_code: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[tokio::test]
// 意图：验证URL HTTPS 校验场景（proto_url_01_https_valid）。预期通过协议校验，因为该输入会命中对应业务分支；不应误报 resource_invalid，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_url_01_https_valid() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Url {
        url: "https://example.com/a.png".to_string(),
        mime_hint: None,
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_ne failed in proto_url_01_https_valid: expected left != right; check validation/policy branch and unexpected equality.");
}

#[tokio::test]
// 意图：验证URL HTTP 策略场景（proto_url_02_http_allowed）。预期在当前策略下通过，因为该输入会命中对应业务分支；不应被错误拦截，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_url_02_http_allowed() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Url {
        url: "http://example.com/a.png".to_string(),
        mime_hint: None,
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_ne failed in proto_url_02_http_allowed: expected left != right; check validation/policy branch and unexpected equality.");
}

#[tokio::test]
// 意图：验证Base64 合法资源场景（proto_b64_01_image_valid_png）。预期通过资源校验，因为该输入会命中对应业务分支；不应误报 resource_invalid，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_b64_01_image_valid_png() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_ne failed in proto_b64_01_image_valid_png: expected left != right; check validation/policy branch and unexpected equality.");
}

#[tokio::test]
// 意图：验证Base64 合法资源场景（proto_b64_02_image_valid_jpeg）。预期通过资源校验，因为该输入会命中对应业务分支；不应误报 resource_invalid，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_b64_02_image_valid_jpeg() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "image/jpeg".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_ne failed in proto_b64_02_image_valid_jpeg: expected left != right; check validation/policy branch and unexpected equality.");
}

#[tokio::test]
// 意图：验证Base64 合法资源场景（proto_b64_03_audio_valid_wav）。预期通过资源校验，因为该输入会命中对应业务分支；不应误报 resource_invalid，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_b64_03_audio_valid_wav() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "audio/wav".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_ne failed in proto_b64_03_audio_valid_wav: expected left != right; check validation/policy branch and unexpected equality.");
}

#[tokio::test]
// 意图：验证Base64 合法资源场景（proto_b64_04_audio_valid_mp3）。预期通过资源校验，因为该输入会命中对应业务分支；不应误报 resource_invalid，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_b64_04_audio_valid_mp3() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "audio/mpeg".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_ne failed in proto_b64_04_audio_valid_mp3: expected left != right; check validation/policy branch and unexpected equality.");
}

#[tokio::test]
// 意图：验证Base64 合法资源场景（proto_b64_05_video_valid_mp4）。预期通过资源校验，因为该输入会命中对应业务分支；不应误报 resource_invalid，否则说明路由、状态机或错误分类逻辑与契约不一致。
async fn proto_b64_05_video_valid_mp4() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "video/mp4".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"), "assert_ne failed in proto_b64_05_video_valid_mp4: expected left != right; check validation/policy branch and unexpected equality.");
}

#[tokio::test]
// 意图：验证错误事件类型场景（task_04_emit_error_event_has_error_kind）。预期出现 TaskEventKind::Error，因为该输入会命中对应业务分支；不应只有普通状态事件，否则说明路由、状态机或错误分类逻辑与契约不一致。
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

#[tokio::test]
// 意图：验证租户映射覆盖全局场景（route_08_tenant_mapping_override_global_on_complete）。预期使用 tenant 映射模型，因为该输入会命中对应业务分支；不应回退到 global 映射，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
