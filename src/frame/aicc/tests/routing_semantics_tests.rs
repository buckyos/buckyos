mod common;

use aicc::metadata_resolver::{resolve_driver_inventory, DriverModelResolveRequest};
use aicc::model_types::{ApiType, ProviderType};
use aicc::{CostEstimate, ModelCatalog, ProviderStartResult, Registry, Router, TenantRouteConfig};
use buckyos_api::{
    AiMessage, AiMethodRequest, AiMethodStatus, AiPayload, AiRole, AiccLogicalNodeOverlay,
    AiccRouteOverlay, Capability, LlmChatInvokeRequest, ModelDisable, ModelItem, ModelSpec,
    Requirements, RouteResolveRequest, TaskFilter, TextToImageInvokeRequest,
};
use common::*;
use std::collections::BTreeMap;
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

fn add_llm(
    registry: &Registry,
    catalog: &ModelCatalog,
    instance_id: &str,
    provider_type: &str,
    cost: f64,
    latency_ms: u64,
    result: std::result::Result<ProviderStartResult, aicc::ProviderError>,
) -> Arc<MockProvider> {
    catalog.set_mapping(Capability::Llm, "llm.plan.default", provider_type, "m");
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
        vec![result],
    ));
    registry.add_provider(provider.clone());
    provider
}

fn add_image(
    registry: &Registry,
    catalog: &ModelCatalog,
    instance_id: &str,
    provider_type: &str,
    cost: f64,
    latency_ms: u64,
    result: std::result::Result<ProviderStartResult, aicc::ProviderError>,
) -> Arc<MockProvider> {
    catalog.set_mapping(
        Capability::Image,
        "image.txt2img.default",
        provider_type,
        "m",
    );
    let provider = Arc::new(MockProvider::new(
        mock_instance(instance_id, provider_type, vec![Capability::Image], vec![]),
        CostEstimate {
            estimated_cost_usd: Some(cost),
            estimated_latency_ms: Some(latency_ms),
        },
        vec![result],
    ));
    registry.add_provider(provider.clone());
    provider
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
    let snapshot = registry.snapshot(Capability::Llm);
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
            vec![Capability::Llm],
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
    let snapshot = registry.snapshot(Capability::Llm);
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
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    let provider = Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::Llm],
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
    let snapshot = registry.snapshot(Capability::Llm);
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
    let snapshot = registry.snapshot(Capability::Llm);
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
    let snapshot = registry.snapshot(Capability::Llm);
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
    let snapshot = registry.snapshot(Capability::Llm);
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
    let snapshot = registry.snapshot(Capability::Llm);
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
        Capability::Llm,
        "llm.plan.default",
        "provider-a",
        "m-tenant",
    );

    let req = base_request();
    let snapshot = registry.snapshot(Capability::Llm);
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
        Capability::Llm,
        "llm.plan.default",
        "provider-a",
        "global-model",
    );
    catalog.set_tenant_mapping(
        "tenant-x",
        Capability::Llm,
        "llm.plan.default",
        "provider-a",
        "tenant-model",
    );
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::Llm],
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
        .list_tasks(None::<TaskFilter>)
        .await
        .expect("list tasks");
    let task = tasks
        .into_iter()
        .find(|t| typed_aicc_external_task_id(t).as_deref() == Some(response.task_id.as_str()))
        .expect("task should exist");
    assert_eq!(
        typed_aicc_task_data(&task)
            .and_then(|data| data.request.route)
            .and_then(|route| route.pointer("/provider_model").cloned())
            .and_then(|v| v.as_str().map(ToString::to_string))
            .as_deref(),
        Some("tenant-model")
    , "assert_eq failed in route_08_tenant_mapping_override_global_on_complete: expected left == right; check this scenario's routing/status/error-code branch.");
}

#[test]
fn route_resolve_returns_control_plane_selection_without_starting_provider() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let provider = add_llm(
        &registry,
        &catalog,
        "p-a",
        "provider-a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &registry,
        &catalog,
        "p-b",
        "provider-b",
        0.02,
        120,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .resolve_route(
            RouteResolveRequest {
                request_id: Some("route-test-1".to_string()),
                api_type: "llm".to_string(),
                logical_model: "llm.plan.default".to_string(),
                requirements: Default::default(),
                disable: Default::default(),
                policy: None,
                estimated_input_tokens: Some(12),
                estimated_output_tokens: Some(24),
                session_overlay: None,
            },
            Default::default(),
        )
        .expect("route.resolve should succeed");

    assert_eq!(response.selected_exact_model, "m@p-a");
    assert_eq!(response.provider_instance_name, "p-a");
    assert_eq!(response.provider_model_id, "m");
    assert_eq!(response.fallback_attempts.len(), 1);
    assert_eq!(response.fallback_attempts[0].exact_model, "m@p-b");
    assert!(response
        .enabled_capabilities
        .iter()
        .any(|capability| capability == buckyos_api::features::WEB_SEARCH));
    assert!(response.disabled_capabilities.is_empty());
    assert_eq!(provider.start_calls(), 0);
}

#[test]
fn route_resolve_applies_request_session_overlay_as_top_config_layer() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let p1 = Arc::new(MockProvider::new(
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
    ));
    let p2 = Arc::new(MockProvider::new(
        mock_instance(
            "p-b",
            "provider-b",
            vec![Capability::Llm],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.02),
            estimated_latency_ms: Some(120),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(p1);
    registry.add_provider(p2);
    let center = center_with_taskmgr(registry, catalog);

    let overlay = AiccRouteOverlay {
        logical_tree: [(
            "llm".to_string(),
            AiccLogicalNodeOverlay {
                children: [(
                    "plan".to_string(),
                    AiccLogicalNodeOverlay {
                        children: [(
                            "default".to_string(),
                            AiccLogicalNodeOverlay {
                                items: Some(
                                    [("only".to_string(), ModelItem::new("m@p-b", 1.0))]
                                        .into_iter()
                                        .collect::<BTreeMap<_, _>>(),
                                ),
                                ..Default::default()
                            },
                        )]
                        .into_iter()
                        .collect(),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };

    let response = center
        .resolve_route(
            RouteResolveRequest {
                request_id: Some("route-overlay-test".to_string()),
                api_type: "llm".to_string(),
                logical_model: "llm.plan.default".to_string(),
                requirements: Default::default(),
                disable: Default::default(),
                policy: None,
                estimated_input_tokens: Some(12),
                estimated_output_tokens: Some(24),
                session_overlay: Some(overlay),
            },
            Default::default(),
        )
        .expect("route.resolve should apply request overlay");

    assert_eq!(response.provider_instance_name, "p-b");
    assert_eq!(response.selected_exact_model, "m@p-b");
}

#[test]
fn route_resolve_rejects_exact_model_input() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    add_llm(
        &registry,
        &catalog,
        "p-a",
        "provider-a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(registry, catalog);

    let err = center
        .resolve_route(
            RouteResolveRequest {
                request_id: Some("route-test-exact".to_string()),
                api_type: "llm".to_string(),
                logical_model: "m@p-a".to_string(),
                requirements: Default::default(),
                disable: Default::default(),
                policy: None,
                estimated_input_tokens: None,
                estimated_output_tokens: None,
                session_overlay: None,
            },
            Default::default(),
        )
        .expect_err("route.resolve should reject exact model names");

    assert!(err
        .to_string()
        .contains("logical_model must be a logical model name"));
}

#[tokio::test]
async fn chat_completions_create_uses_exact_model_without_runtime_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let primary = add_llm(
        &registry,
        &catalog,
        "p-a",
        "provider-a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    let fallback = add_llm(
        &registry,
        &catalog,
        "p-b",
        "provider-b",
        0.02,
        90,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .create_chat_completion(
            LlmChatInvokeRequest {
                exact_model: "m@p-a".to_string(),
                messages: vec![AiMessage::text(AiRole::User, "hello")],
                tools: vec![],
                response_format: None,
                temperature: None,
                max_output_tokens: None,
                payload: None,
                provider_options: None,
                idempotency_key: None,
                task_options: None,
            },
            Default::default(),
        )
        .await
        .expect("chat.completions.create should start exact provider");

    assert_eq!(response.status, AiMethodStatus::Running);
    assert_eq!(primary.start_calls(), 1);
    assert_eq!(fallback.start_calls(), 0);
}

#[tokio::test]
async fn chat_completions_create_rejects_logical_model_name() {
    let center = center_with_taskmgr(Registry::default(), ModelCatalog::default());

    let err = center
        .create_chat_completion(
            LlmChatInvokeRequest {
                exact_model: "llm.plan.default".to_string(),
                messages: vec![AiMessage::text(AiRole::User, "hello")],
                tools: vec![],
                response_format: None,
                temperature: None,
                max_output_tokens: None,
                payload: None,
                provider_options: None,
                idempotency_key: None,
                task_options: None,
            },
            Default::default(),
        )
        .await
        .expect_err("chat.completions.create should reject logical model names");

    assert!(err
        .to_string()
        .contains("exact model name must contain provider instance suffix"));
}

#[tokio::test]
async fn images_generate_rejects_logical_model_name() {
    let center = center_with_taskmgr(Registry::default(), ModelCatalog::default());

    let err = center
        .generate_image(
            TextToImageInvokeRequest {
                exact_model: "image.txt2img.default".to_string(),
                prompt: "draw a cube".to_string(),
                negative_prompt: None,
                size: None,
                quality: None,
                style: None,
                seed: None,
                output: None,
                payload: None,
                provider_options: None,
                idempotency_key: None,
                task_options: None,
            },
            Default::default(),
        )
        .await
        .expect_err("images.generate should reject logical model names");

    assert!(err
        .to_string()
        .contains("exact model name must contain provider instance suffix"));
}

#[tokio::test]
async fn typed_exact_unavailable_does_not_fallback_to_other_model() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let fallback = add_llm(
        &registry,
        &catalog,
        "p-b",
        "provider-b",
        0.02,
        90,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .create_chat_completion(
            LlmChatInvokeRequest {
                exact_model: "missing@p-a".to_string(),
                messages: vec![AiMessage::text(AiRole::User, "hello")],
                tools: vec![],
                response_format: None,
                temperature: None,
                max_output_tokens: None,
                payload: None,
                provider_options: None,
                idempotency_key: None,
                task_options: None,
            },
            Default::default(),
        )
        .await
        .expect("typed inference reports routing failure as failed task response");

    assert_eq!(response.status, AiMethodStatus::Failed);
    assert_eq!(fallback.start_calls(), 0);
}

#[tokio::test]
async fn helper_llm_chat_expands_to_route_resolve_and_typed_inference() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let provider = add_llm(
        &registry,
        &catalog,
        "p-a",
        "provider-a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(registry, catalog);
    let request = AiMethodRequest::new(
        Capability::Llm,
        ModelSpec::new("llm.plan.default".to_string(), None),
        Requirements::default(),
        AiPayload::new(
            None,
            vec![AiMessage::text(AiRole::User, "hello")],
            vec![],
            vec![],
            None,
            None,
        ),
        None,
    );

    let response = center
        .helper_llm_chat(request, Default::default())
        .await
        .expect("helper.llm_chat should succeed through two-stage flow");

    assert_eq!(response.status, AiMethodStatus::Running);
    assert_eq!(provider.start_calls(), 1);
}

#[tokio::test]
async fn helper_text_to_image_expands_to_route_resolve_and_typed_inference() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let provider = add_image(
        &registry,
        &catalog,
        "p-img",
        "provider-img",
        0.02,
        300,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(registry, catalog);
    let request = AiMethodRequest::new(
        Capability::Image,
        ModelSpec::new("image.txt2img.default".to_string(), None),
        Requirements::default(),
        AiPayload::new(
            Some("draw a cube".to_string()),
            vec![],
            vec![],
            vec![],
            Some(serde_json::json!({
                "prompt": "draw a cube",
                "size": "1024x1024"
            })),
            None,
        ),
        None,
    );

    let response = center
        .helper_text_to_image(request, Default::default())
        .await
        .expect("helper.text_to_image should succeed through two-stage flow");

    assert_eq!(response.status, AiMethodStatus::Running);
    assert_eq!(provider.start_calls(), 1);
}

#[test]
fn openai_resolver_expands_reasoning_variants() {
    let inventory = resolve_driver_inventory(
        "openai-primary",
        ProviderType::CloudApi,
        "openai",
        &[DriverModelResolveRequest::new("gpt-5.1", vec![])],
        Some("test".to_string()),
    );

    let high = inventory
        .models
        .iter()
        .find(|model| model.provider_model_id == "gpt-5.1:reasoning-high")
        .expect("reasoning-high variant should exist");

    assert_eq!(high.exact_model, "gpt-5.1:reasoning-high@openai-primary");
    assert_eq!(high.provider_actual_model_id.as_deref(), Some("gpt-5.1"));
    assert_eq!(
        high.provider_options
            .as_ref()
            .and_then(|options| options.pointer("/reasoning/effort"))
            .and_then(|value| value.as_str()),
        Some("high")
    );
    assert!(high
        .logical_mounts
        .iter()
        .any(|mount| mount == "llm.openai.gpt-5-1.reasoning-high"));
}

#[test]
fn purpose_logical_route_reaches_driver_family_mount() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let instance = mock_instance(
        "google-gemini-main",
        "google-gemini",
        vec![Capability::Llm],
        vec!["plan".to_string()],
    );
    let inventory = resolve_driver_inventory(
        "google-gemini-main",
        ProviderType::CloudApi,
        "google-gemini",
        &[DriverModelResolveRequest::new(
            "gemini-2.5-pro",
            vec![ApiType::Llm],
        )],
        Some("test".to_string()),
    );
    let model = inventory
        .models
        .iter()
        .find(|model| model.provider_model_id == "gemini-2.5-pro")
        .expect("gemini pro model should resolve");
    assert!(model
        .logical_mounts
        .iter()
        .any(|mount| mount == "llm.gemini-pro"));
    assert!(!model.logical_mounts.iter().any(|mount| mount == "llm.plan"));

    let provider = Arc::new(MockProvider::with_inventory(
        instance,
        inventory,
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(provider);
    let center = center_with_taskmgr(registry, catalog);
    let mut disable = ModelDisable::default();
    disable.web_search = true;

    let response = center
        .resolve_route(
            RouteResolveRequest {
                request_id: Some("family-route".to_string()),
                api_type: "llm".to_string(),
                logical_model: "llm.plan".to_string(),
                requirements: Requirements::default(),
                disable,
                policy: None,
                estimated_input_tokens: None,
                estimated_output_tokens: None,
                session_overlay: None,
            },
            Default::default(),
        )
        .expect("llm.plan should route through llm.gemini-pro family mount");

    assert_eq!(
        response.selected_exact_model,
        "gemini-2.5-pro@google-gemini-main"
    );
}

#[test]
fn route_resolve_outputs_base_provider_model_and_variant_options() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let instance = mock_instance(
        "openai-primary",
        "openai",
        vec![Capability::Llm],
        vec!["plan".to_string()],
    );
    let inventory = resolve_driver_inventory(
        "openai-primary",
        ProviderType::CloudApi,
        "openai",
        &[DriverModelResolveRequest::new("gpt-5.1", vec![])],
        Some("test".to_string()),
    );
    let provider = Arc::new(MockProvider::with_inventory(
        instance,
        inventory,
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(provider);
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .resolve_route(
            RouteResolveRequest {
                request_id: Some("test-route".to_string()),
                api_type: "llm".to_string(),
                logical_model: "llm.openai.gpt-5-1.reasoning-high".to_string(),
                requirements: Requirements::default(),
                disable: Default::default(),
                policy: None,
                estimated_input_tokens: None,
                estimated_output_tokens: None,
                session_overlay: None,
            },
            Default::default(),
        )
        .expect("route.resolve should select variant logical mount");

    assert_eq!(
        response.selected_exact_model,
        "gpt-5.1:reasoning-high@openai-primary"
    );
    assert_eq!(response.provider_model_id, "gpt-5.1");
    assert_eq!(
        response
            .provider_options
            .as_ref()
            .and_then(|options| options.pointer("/reasoning/effort"))
            .and_then(|value| value.as_str()),
        Some("high")
    );
    assert_eq!(
        response
            .route_trace
            .as_ref()
            .and_then(|trace| trace.get("selected_provider_model_id"))
            .and_then(|value| value.as_str()),
        Some("gpt-5.1")
    );
}

#[tokio::test]
async fn typed_variant_exact_model_lowers_to_provider_base_and_options() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let instance = mock_instance(
        "openai-primary",
        "openai",
        vec![Capability::Llm],
        vec!["plan".to_string()],
    );
    let inventory = resolve_driver_inventory(
        "openai-primary",
        ProviderType::CloudApi,
        "openai",
        &[DriverModelResolveRequest::new("gpt-5.1", vec![])],
        Some("test".to_string()),
    );
    let provider = Arc::new(MockProvider::with_inventory(
        instance,
        inventory,
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(provider.clone());
    let center = center_with_taskmgr(registry, catalog);

    let response = center
        .create_chat_completion(
            LlmChatInvokeRequest {
                exact_model: "gpt-5.1:reasoning-high@openai-primary".to_string(),
                messages: vec![AiMessage::text(AiRole::User, "hello")],
                tools: vec![],
                response_format: None,
                temperature: None,
                max_output_tokens: None,
                payload: None,
                provider_options: None,
                idempotency_key: None,
                task_options: None,
            },
            Default::default(),
        )
        .await
        .expect("typed exact variant should start");

    assert_eq!(response.status, AiMethodStatus::Running);
    assert_eq!(provider.last_provider_model().as_deref(), Some("gpt-5.1"));
    assert_eq!(
        provider
            .last_request_options()
            .as_ref()
            .and_then(|options| options.pointer("/provider_options/reasoning/effort"))
            .and_then(|value| value.as_str()),
        Some("high")
    );
}
