mod common;

use aicc::{CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry};
use buckyos_api::{AiMethodStatus, Capability};
use common::*;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
// 用例说明：
// - 验证场景：`start_01_retryable_error_then_fallback_success` 用例，覆盖可重试错误分支、回退策略分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果。
// - 处理流程：调用 complete/start 路径，按主实例到回退实例的顺序进行启动尝试与状态推进。
// - 预期输出：返回成功结果，关键字段与断言一致；错误被归类为可重试并触发对应策略；回退执行次数与顺序满足用例断言。
async fn start_01_retryable_error_then_fallback_success() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-b", "m-b");
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
        vec![Err(ProviderError::retryable("temp"))],
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
    assert_eq!(
        response.status,
        AiMethodStatus::Running,
        "assert_eq failed in start_01_retryable_error_then_fallback_success: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        p1.start_calls(),
        1,
        "assert_eq failed in start_01_retryable_error_then_fallback_success: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        p2.start_calls(),
        1,
        "assert_eq failed in start_01_retryable_error_then_fallback_success: expected left == right; check this scenario's routing/status/error-code branch."
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`start_02_fatal_error_no_fallback` 用例，覆盖致命错误分支、回退策略分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果。
// - 处理流程：调用 complete/start 路径，按主实例到回退实例的顺序进行启动尝试与状态推进。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期；不会发生额外回退尝试。
async fn start_02_fatal_error_no_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-b", "m-b");
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
        vec![Err(ProviderError::fatal("bad request"))],
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
    assert_eq!(
        response.status,
        AiMethodStatus::Failed,
        "assert_eq failed in start_02_fatal_error_no_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        p1.start_calls(),
        1,
        "assert_eq failed in start_02_fatal_error_no_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        p2.start_calls(),
        0,
        "assert_eq failed in start_02_fatal_error_no_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
}

#[tokio::test]
async fn start_retryable_error_respects_runtime_failover_false() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-b", "m-b");
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
        vec![Err(ProviderError::retryable("temp"))],
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
            estimated_latency_ms: Some(200),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(p1.clone());
    registry.add_provider(p2.clone());
    let center = center_with_taskmgr(registry, catalog);

    let mut req = base_request();
    req.requirements.extra = Some(json!({"runtime_failover": false}));
    let response = center
        .complete(req, rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .expect("complete should return");

    assert_eq!(response.status, AiMethodStatus::Failed);
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 0);
}

#[tokio::test]
async fn request_session_config_patch_updates_session_policy() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-b", "m-b");
    let slow_cheap = Arc::new(MockProvider::new(
        mock_instance(
            "p-a",
            "provider-a",
            vec![Capability::Llm],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.001),
            estimated_latency_ms: Some(1000),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    let fast = Arc::new(MockProvider::new(
        mock_instance(
            "p-b",
            "provider-b",
            vec![Capability::Llm],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.002),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Started)],
    ));
    registry.add_provider(slow_cheap.clone());
    registry.add_provider(fast.clone());
    let center = center_with_taskmgr(registry, catalog);

    let mut req = base_request();
    req.payload.options = Some(json!({"session_id": "s-policy"}));
    req.requirements.extra = Some(json!({
        "session_config_patch": {
            "policy": {
                "profile": "latency_first"
            }
        }
    }));
    let response = center
        .complete(req, rpc_ctx_with_tenant(Some("tenant-a")))
        .await
        .expect("complete should return");

    assert_eq!(response.status, AiMethodStatus::Running);
    assert_eq!(slow_cheap.start_calls(), 0);
    assert_eq!(fast.start_calls(), 1);
}

#[tokio::test]
// 用例说明：
// - 验证场景：`start_03_started_must_stop_fallback` 用例，覆盖回退策略分支、启动成功状态分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果。
// - 处理流程：调用 complete/start 路径，按主实例到回退实例的顺序进行启动尝试与状态推进。
// - 预期输出：不会发生额外回退尝试。
async fn start_03_started_must_stop_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-b", "m-b");
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
    assert_eq!(
        response.status,
        AiMethodStatus::Running,
        "assert_eq failed in start_03_started_must_stop_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        p1.start_calls(),
        1,
        "assert_eq failed in start_03_started_must_stop_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        p2.start_calls(),
        0,
        "assert_eq failed in start_03_started_must_stop_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`start_04_queued_no_fallback` 用例，覆盖回退策略分支、排队状态分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果。
// - 处理流程：调用 complete/start 路径，按主实例到回退实例的顺序进行启动尝试与状态推进。
// - 预期输出：不会发生额外回退尝试。
async fn start_04_queued_no_fallback() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-b", "m-b");
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
        vec![Ok(ProviderStartResult::Queued { position: 2 })],
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
    assert_eq!(
        response.status,
        AiMethodStatus::Running,
        "assert_eq failed in start_04_queued_no_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        p1.start_calls(),
        1,
        "assert_eq failed in start_04_queued_no_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        p2.start_calls(),
        0,
        "assert_eq failed in start_04_queued_no_fallback: expected left == right; check this scenario's routing/status/error-code branch."
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`start_05_all_candidates_failed_provider_start_failed` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：调用 complete/start 路径，按主实例到回退实例的顺序进行启动尝试与状态推进。
// - 预期输出：最终错误为 provider_start_failed。
async fn start_05_all_candidates_failed_provider_start_failed() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-a", "m-a");
    catalog.set_mapping(Capability::Llm, "llm.plan.default", "provider-b", "m-b");
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
        vec![Err(ProviderError::retryable("retry-a"))],
    )));
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-b",
            "provider-b",
            vec![Capability::Llm],
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
    assert_eq!(
        response.status,
        AiMethodStatus::Failed,
        "assert_eq failed in start_05_all_candidates_failed_provider_start_failed: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        extract_error_code(&sink.events_for(&response.task_id)).as_deref(),
        Some("provider_start_failed"),
        "assert_eq failed in start_05_all_candidates_failed_provider_start_failed: expected left == right; check this scenario's routing/status/error-code branch."
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`start_06_fallback_respects_limit` 用例，覆盖回退策略分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果。
// - 处理流程：调用 complete/start 路径，按主实例到回退实例的顺序进行启动尝试与状态推进。
// - 预期输出：回退执行次数与顺序满足用例断言。
async fn start_06_fallback_respects_limit() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    for (id, ptype, model, cost) in [
        ("p-a", "provider-a", "m-a", 0.01),
        ("p-b", "provider-b", "m-b", 0.02),
        ("p-c", "provider-c", "m-c", 0.03),
        ("p-d", "provider-d", "m-d", 0.04),
    ] {
        catalog.set_mapping(Capability::Llm, "llm.plan.default", ptype, model);
        registry.add_provider(Arc::new(MockProvider::new(
            mock_instance(id, ptype, vec![Capability::Llm], vec!["plan".into()]),
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
    assert_eq!(
        response.status,
        AiMethodStatus::Failed,
        "assert_eq failed in start_06_fallback_respects_limit: expected left == right; check this scenario's routing/status/error-code branch."
    );
    assert_eq!(
        extract_error_code(&sink.events_for(&response.task_id)).as_deref(),
        Some("provider_start_failed"),
        "assert_eq failed in start_06_fallback_respects_limit: expected left == right; check this scenario's routing/status/error-code branch."
    );
}
