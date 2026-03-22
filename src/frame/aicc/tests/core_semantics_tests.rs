mod common;

use aicc::{
    AIComputeCenter, CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry, Router,
    TaskEventKind, TenantRouteConfig,
};
use buckyos_api::{AiResponseSummary, Capability, CompleteStatus, ResourceRef, TaskFilter, TaskStatus};
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

    assert_eq!(decision.primary_instance_id, "p-a");
    assert_eq!(decision.fallback_instance_ids, vec!["p-b".to_string()]);
}

#[test]
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
    assert!(err.to_string().contains("model_alias_not_mapped"));
}

#[test]
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
    assert!(err.to_string().contains("no_provider_available"));
}

#[test]
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
    assert_eq!(decision.primary_instance_id, "p-a");
}

#[test]
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
    assert_eq!(decision.primary_instance_id, "p-a");
}

#[test]
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
    assert!(err.to_string().contains("no_provider_available"));
}

#[test]
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
    assert!(err.to_string().contains("no_provider_available"));
}

#[test]
fn route_08_tenant_mapping_override_global() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p-a", "provider-a", "m-global", 0.01, 100);
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
    assert_eq!(decision.provider_model, "m-tenant");
}

#[tokio::test]
async fn start_01_retryable_error_then_fallback_success() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-b", "m-b");
    let p1 = Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Err(ProviderError::retryable("temp"))],
    ));
    let p2 = Arc::new(MockProvider::new(
        mock_instance("p-b", "provider-b", vec![Capability::LlmRouter], vec!["plan".into()]),
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
    assert_eq!(response.status, CompleteStatus::Running);
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 1);
}

#[tokio::test]
async fn start_02_fatal_error_no_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-b", "m-b");
    let p1 = Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Err(ProviderError::fatal("bad request"))],
    ));
    let p2 = Arc::new(MockProvider::new(
        mock_instance("p-b", "provider-b", vec![Capability::LlmRouter], vec!["plan".into()]),
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
    assert_eq!(response.status, CompleteStatus::Failed);
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 0);
}

#[tokio::test]
async fn start_03_started_must_stop_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-b", "m-b");
    let p1 = Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    let p2 = Arc::new(MockProvider::new(
        mock_instance("p-b", "provider-b", vec![Capability::LlmRouter], vec!["plan".into()]),
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
    assert_eq!(response.status, CompleteStatus::Running);
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 0);
}

#[tokio::test]
async fn start_04_queued_no_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-b", "m-b");
    let p1 = Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Queued { position: 2 })],
    ));
    let p2 = Arc::new(MockProvider::new(
        mock_instance("p-b", "provider-b", vec![Capability::LlmRouter], vec!["plan".into()]),
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
    assert_eq!(response.status, CompleteStatus::Running);
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 0);
}

#[tokio::test]
async fn start_05_all_candidates_failed_provider_start_failed() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-b", "m-b");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Err(ProviderError::retryable("retry-a"))],
    )));
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-b", "provider-b", vec![Capability::LlmRouter], vec!["plan".into()]),
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
    assert_eq!(response.status, CompleteStatus::Failed);
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("provider_start_failed"));
}

#[tokio::test]
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
    assert_eq!(response.status, CompleteStatus::Failed);
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("provider_start_failed"));
}

#[tokio::test]
async fn task_01_immediate_persists_completed() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
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

    let response = center.complete(base_request(), rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(response.status, CompleteStatus::Succeeded);

    let taskmgr = center.task_manager_client().expect("task manager");
    let tasks = taskmgr
        .list_tasks(None::<TaskFilter>, None, None)
        .await
        .expect("list tasks");
    let task = tasks
        .into_iter()
        .find(|t| {
            t.data.pointer("/aicc/external_task_id").and_then(|v| v.as_str())
                == Some(response.task_id.as_str())
        })
        .expect("task should exist");
    assert_eq!(task.status, TaskStatus::Completed);
}

#[tokio::test]
async fn task_02_started_persists_running_and_binding() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    let provider = Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
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
    assert_eq!(response.status, CompleteStatus::Running);
    let cancel = center
        .cancel(response.task_id.as_str(), rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .unwrap();
    assert!(cancel.accepted);
    assert_eq!(provider.canceled_tasks(), vec![response.task_id]);
}

#[tokio::test]
async fn task_03_queued_persists_pending_and_position() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Queued { position: 7 })],
    )));
    let center = center_with_taskmgr(registry, catalog);

    let response = center.complete(base_request(), rpc_ctx_with_tenant(None)).await.unwrap();
    let taskmgr = center.task_manager_client().expect("task manager");
    let tasks = taskmgr
        .list_tasks(None::<TaskFilter>, None, None)
        .await
        .expect("list tasks");
    let task = tasks
        .into_iter()
        .find(|t| {
            t.data.pointer("/aicc/external_task_id").and_then(|v| v.as_str())
                == Some(response.task_id.as_str())
        })
        .expect("task should exist");
    assert_eq!(task.status, TaskStatus::Pending);
    assert_eq!(
        task.data.pointer("/aicc/events/0/kind").and_then(|v| v.as_str()),
        Some("queued")
    );
}

#[tokio::test]
async fn task_04_emit_error_event_with_code() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());

    let mut req = base_request();
    req.model.alias = "   ".to_string();
    let response = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(response.status, CompleteStatus::Failed);
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("bad_request"));
}

#[tokio::test]
async fn sec_01_cancel_reject_cross_tenant() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
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
        .cancel(start.task_id.as_str(), rpc_ctx_with_tenant(Some("tenant-bob")))
        .await
        .expect_err("cross tenant cancel must fail");
    assert!(err.to_string().contains("NoPermission") || err.to_string().contains("cross-tenant"));
}

#[tokio::test]
async fn sec_02_cancel_accept_same_tenant() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
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
        .cancel(start.task_id.as_str(), rpc_ctx_with_tenant(Some("tenant-alice")))
        .await
        .unwrap();
    assert!(resp.accepted);
}

#[tokio::test]
async fn sec_03_resource_invalid_from_resolver() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    center.set_resource_resolver(Arc::new(FailingResolver {
        message: "resolver denied".to_string(),
    }));
    center.registry().add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    center
        .model_catalog()
        .set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");

    let response = center.complete(base_request(), rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(response.status, CompleteStatus::Failed);
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
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
    let response = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(response.status, CompleteStatus::Failed);
    assert_eq!(extract_error_code(&sink.events_for(&response.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn obs_01_error_code_mapping_consistent() {
    let sink = Arc::new(CollectingSinkFactory::new());

    let center_bad = {
        let mut c = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
        c.set_task_event_sink_factory(sink.clone());
        c
    };
    let mut bad_req = base_request();
    bad_req.model.alias = "".to_string();
    let bad_resp = center_bad.complete(bad_req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&bad_resp.task_id)).as_deref(), Some("bad_request"));

    let center_no_provider = {
        let mut c = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
        c.set_task_event_sink_factory(sink.clone());
        c
    };
    let no_provider_resp = center_no_provider
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&no_provider_resp.task_id)).as_deref(), Some("no_provider_available"));
}

#[tokio::test]
async fn obs_02_log_redaction_no_prompt_or_base64() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let secret = "very-sensitive-base64";
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: secret.to_string(),
    });
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    let events = sink.events_for(&resp.task_id);
    let msg = events
        .iter()
        .filter_map(|e| e.data.as_ref())
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!msg.contains(secret));
}

#[tokio::test]
async fn conc_01_task_id_uniqueness_under_concurrency() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: Some("ok".to_string()),
            ..Default::default()
        })); 128],
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
    assert_eq!(ids.len(), 64);
}

#[tokio::test]
async fn conc_02_registry_hot_update_route_consistency() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
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
async fn proto_b64_06_invalid_mime_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/gif".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    });
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_b64_07_size_limit_exceeded_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    center.set_base64_policy(2, string_set(&["image/png"]));
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(&[1, 2, 3, 4]),
    });
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_b64_08_malformed_base64_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: "%%%not-base64%%%".to_string(),
    });
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_url_03_missing_scheme_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Url {
        url: "example.com/image.png".to_string(),
        mime_hint: None,
    });
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_url_04_empty_url_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Url {
        url: " ".to_string(),
        mime_hint: None,
    });
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_sec_04_idempotency_key_preserved() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let center = center_with_taskmgr(registry, catalog);
    let req = base_request();
    let idem = req.idempotency_key.clone().expect("idem key");
    let response = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    let taskmgr = center.task_manager_client().expect("task manager");
    let tasks = taskmgr
        .list_tasks(None::<TaskFilter>, None, None)
        .await
        .expect("list tasks");
    let task = tasks
        .into_iter()
        .find(|t| {
            t.data.pointer("/aicc/external_task_id").and_then(|v| v.as_str())
                == Some(response.task_id.as_str())
        })
        .expect("task should exist");
    assert_eq!(
        task.data
            .pointer("/aicc/request/idempotency_key")
            .and_then(|v| v.as_str()),
        Some(idem.as_str())
    );
}

#[tokio::test]
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
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn obs_01_provider_start_failed_code() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m-a");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(100),
        },
        vec![Err(ProviderError::fatal("boom"))],
    )));
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    let resp = center.complete(base_request(), rpc_ctx_with_tenant(None)).await.unwrap();
    assert_eq!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("provider_start_failed"));
}

#[tokio::test]
async fn proto_url_01_https_valid() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Url {
        url: "https://example.com/a.png".to_string(),
        mime_hint: None,
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_url_02_http_allowed() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Url {
        url: "http://example.com/a.png".to_string(),
        mime_hint: None,
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_b64_01_image_valid_png() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_b64_02_image_valid_jpeg() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "image/jpeg".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_b64_03_audio_valid_wav() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "audio/wav".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_b64_04_audio_valid_mp3() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "audio/mpeg".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn proto_b64_05_video_valid_mp4() {
    let mut req = base_request();
    req.payload.resources = vec![ResourceRef::Base64 {
        mime: "video/mp4".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let resp = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    assert_ne!(extract_error_code(&sink.events_for(&resp.task_id)).as_deref(), Some("resource_invalid"));
}

#[tokio::test]
async fn task_04_emit_error_event_has_error_kind() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let mut req = base_request();
    req.model.alias = "".to_string();
    let response = center.complete(req, rpc_ctx_with_tenant(None)).await.unwrap();
    let events = sink.events_for(&response.task_id);
    assert!(events.iter().any(|e| matches!(e.kind, TaskEventKind::Error)));
}

#[tokio::test]
async fn route_08_tenant_mapping_override_global_on_complete() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "global-model");
    catalog.set_tenant_mapping(
        "tenant-x",
        Capability::LlmRouter,
        "llm.plan.default",
        "provider-a",
        "tenant-model",
    );
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance("p-a", "provider-a", vec![Capability::LlmRouter], vec!["plan".to_string()]),
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
            t.data.pointer("/aicc/external_task_id").and_then(|v| v.as_str())
                == Some(response.task_id.as_str())
        })
        .expect("task should exist");
    assert_eq!(
        task.data
            .pointer("/aicc/route/provider_model")
            .and_then(|v| v.as_str()),
        Some("tenant-model")
    );
}
