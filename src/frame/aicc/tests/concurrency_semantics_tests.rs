mod common;

use aicc::{CostEstimate, ModelCatalog, ProviderStartResult, Registry, Router};
use buckyos_api::{AiResponseSummary, Capability};
use common::*;
use std::sync::Arc;

#[tokio::test]
// 用例说明：
// - 验证场景：`conc_01_task_id_uniqueness_under_concurrency` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：并发执行目标操作，汇总结果后校验唯一性或一致性。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
    assert_eq!(
        ids.len(),
        64,
        "assert_eq failed in conc_01_task_id_uniqueness_under_concurrency: expected left == right; check this scenario's routing/status/error-code branch."
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`conc_02_registry_hot_update_route_consistency` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：并发执行目标操作，汇总结果后校验唯一性或一致性。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
        let decision = result.expect("route should stay stable during updates");
        assert_eq!(decision.primary_instance_id, id);
        assert_eq!(decision.provider_model, "m-a");
        assert!(decision.fallback_instance_ids.is_empty());
        registry.remove_instance(id.as_str());
    }
}
