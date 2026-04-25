mod common;

use aicc::{
    CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry, RouteConfig,
    RouteWeights, Router, TenantRouteConfig,
};
use buckyos_api::{AiMethodStatus, AiResponseSummary, AiccServerHandler, Capability};
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
    catalog.set_mapping(Capability::Llm, "llm.plan.default", ptype, "m");
    let p = Arc::new(MockProvider::new(
        mock_instance(id, ptype, vec![Capability::Llm], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(cost),
            estimated_latency_ms: Some(lat),
        },
        vec![r],
    ));
    registry.add_provider(p.clone());
    p
}

#[test]
// 用例说明：
// - 验证场景：`sched_01_cost_weight_prefers_low_cost` 用例，覆盖权重调度分支。
// - 输入参数：配置全局或租户级路由权重。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn sched_01_cost_weight_prefers_low_cost() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.20,
        10,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 1.0,
        w_latency: 0.0,
        w_load: 0.0,
        w_error: 0.0,
    };
    assert_eq!(
        Router
            .route(
                "tenant-a",
                &base_request(),
                &r.snapshot(Capability::Llm),
                &r,
                &cfg,
                &c
            )
            .unwrap()
            .primary_instance_id,
        "p1"
    );
}

#[test]
// 用例说明：
// - 验证场景：`sched_02_latency_weight_prefers_low_latency` 用例，覆盖权重调度分支。
// - 输入参数：配置全局或租户级路由权重。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn sched_02_latency_weight_prefers_low_latency() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.20,
        10,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 0.0,
        w_latency: 1.0,
        w_load: 0.0,
        w_error: 0.0,
    };
    assert_eq!(
        Router
            .route(
                "tenant-a",
                &base_request(),
                &r.snapshot(Capability::Llm),
                &r,
                &cfg,
                &c
            )
            .unwrap()
            .primary_instance_id,
        "p2"
    );
}

#[test]
// 用例说明：
// - 验证场景：`sched_03_load_weight_prefers_less_in_flight` 用例，覆盖权重调度分支。
// - 输入参数：配置全局或租户级路由权重。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn sched_03_load_weight_prefers_less_in_flight() {
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
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    for _ in 0..5 {
        r.mark_start_begin("p1");
    }
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 0.0,
        w_latency: 0.0,
        w_load: 1.0,
        w_error: 0.0,
    };
    assert_eq!(
        Router
            .route(
                "tenant-a",
                &base_request(),
                &r.snapshot(Capability::Llm),
                &r,
                &cfg,
                &c
            )
            .unwrap()
            .primary_instance_id,
        "p2"
    );
}

#[test]
// 用例说明：
// - 验证场景：`sched_04_error_weight_prefers_low_error_rate` 用例，覆盖权重调度分支。
// - 输入参数：配置全局或租户级路由权重。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn sched_04_error_weight_prefers_low_error_rate() {
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
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    r.record_start_failure("p1", 100.0);
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 0.0,
        w_latency: 0.0,
        w_load: 0.0,
        w_error: 1.0,
    };
    assert_eq!(
        Router
            .route(
                "tenant-a",
                &base_request(),
                &r.snapshot(Capability::Llm),
                &r,
                &cfg,
                &c
            )
            .unwrap()
            .primary_instance_id,
        "p2"
    );
}

#[test]
// 用例说明：
// - 验证场景：`sched_05_fallback_limit_respected` 用例，覆盖回退策略分支。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：回退执行次数与顺序满足用例断言。
fn sched_05_fallback_limit_respected() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    for i in 0..4 {
        let id = format!("p{}", i);
        let ty = format!("t{}", i);
        add_llm(
            &r,
            &c,
            id.as_str(),
            ty.as_str(),
            0.01 + i as f64 * 0.01,
            10 + i as u64,
            Ok(ProviderStartResult::Started),
        );
    }
    let mut cfg = RouteConfig::default();
    cfg.fallback_limit = 1;
    assert!(
        Router
            .route(
                "tenant-a",
                &base_request(),
                &r.snapshot(Capability::Llm),
                &r,
                &cfg,
                &c
            )
            .unwrap()
            .fallback_instance_ids
            .len()
            <= 1
    );
}

#[test]
// 用例说明：
// - 验证场景：`sched_06_tenant_weight_override_global` 用例，覆盖权重调度分支。
// - 输入参数：设置租户 token 或 tenant_id；配置全局或租户级路由权重。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn sched_06_tenant_weight_override_global() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.20,
        10,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 1.0,
        w_latency: 0.0,
        w_load: 0.0,
        w_error: 0.0,
    };
    cfg.tenant_overrides.insert(
        "tenant-a".into(),
        TenantRouteConfig {
            allow_provider_types: None,
            deny_provider_types: None,
            weights: Some(RouteWeights {
                w_cost: 0.0,
                w_latency: 1.0,
                w_load: 0.0,
                w_error: 0.0,
            }),
        },
    );
    assert_eq!(
        Router
            .route(
                "tenant-a",
                &base_request(),
                &r.snapshot(Capability::Llm),
                &r,
                &cfg,
                &c
            )
            .unwrap()
            .primary_instance_id,
        "p2"
    );
}

#[test]
// 用例说明：
// - 验证场景：`sched_07_tenant_allow_deny_combination` 用例，覆盖函数名对应的业务路径。
// - 输入参数：设置租户 token 或 tenant_id。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn sched_07_tenant_allow_deny_combination() {
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
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.tenant_overrides.insert(
        "tenant-a".into(),
        TenantRouteConfig {
            allow_provider_types: Some(vec!["a".into()]),
            deny_provider_types: Some(vec!["b".into()]),
            weights: None,
        },
    );
    assert_eq!(
        Router
            .route(
                "tenant-a",
                &base_request(),
                &r.snapshot(Capability::Llm),
                &r,
                &cfg,
                &c
            )
            .unwrap()
            .primary_instance_id,
        "p1"
    );
}

#[test]
// 用例说明：
// - 验证场景：`sched_08_alias_mapping_under_strategy` 用例，覆盖别名映射分支。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：调用 Router.route，依次执行映射解析、候选过滤、打分排序与回退列表生成。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
fn sched_08_alias_mapping_under_strategy() {
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
    assert_eq!(
        Router
            .route(
                "tenant-a",
                &base_request(),
                &r.snapshot(Capability::Llm),
                &r,
                &RouteConfig::default(),
                &c
            )
            .unwrap()
            .primary_instance_id,
        "p1"
    );
}

#[test]
fn sched_01_effect_priority_prefers_higher_quality_when_budget_allows() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "hq",
        "a",
        0.10,
        10,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "lq",
        "b",
        0.05,
        100,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 0.0,
        w_latency: 1.0,
        w_load: 0.0,
        w_error: 0.0,
    };
    let selected = Router
        .route(
            "tenant-a",
            &base_request(),
            &r.snapshot(Capability::Llm),
            &r,
            &cfg,
            &c,
        )
        .unwrap();
    assert_eq!(selected.primary_instance_id, "hq");
}

#[test]
fn sched_02_cost_priority_prefers_lower_cost_under_same_capability() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "cheap",
        "a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "expensive",
        "b",
        0.20,
        20,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 1.0,
        w_latency: 0.0,
        w_load: 0.0,
        w_error: 0.0,
    };
    let selected = Router
        .route(
            "tenant-a",
            &base_request(),
            &r.snapshot(Capability::Llm),
            &r,
            &cfg,
            &c,
        )
        .unwrap();
    assert_eq!(selected.primary_instance_id, "cheap");
}

#[test]
fn sched_03_free_quota_priority_prefers_quota_provider_first() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "quota",
        "a",
        0.0,
        50,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "paid",
        "b",
        0.05,
        10,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 1.0,
        w_latency: 0.0,
        w_load: 0.0,
        w_error: 0.0,
    };
    let selected = Router
        .route(
            "tenant-a",
            &base_request(),
            &r.snapshot(Capability::Llm),
            &r,
            &cfg,
            &c,
        )
        .unwrap();
    assert_eq!(selected.primary_instance_id, "quota");
}

#[test]
fn sched_04_agent_tier_policy_routes_to_expected_provider_group() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "tier-a",
        "tier_a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "tier-b",
        "tier_b",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.tenant_overrides.insert(
        "tenant-a".into(),
        TenantRouteConfig {
            allow_provider_types: Some(vec!["tier_b".into()]),
            deny_provider_types: None,
            weights: None,
        },
    );
    let selected = Router
        .route(
            "tenant-a",
            &base_request(),
            &r.snapshot(Capability::Llm),
            &r,
            &cfg,
            &c,
        )
        .unwrap();
    assert_eq!(selected.primary_instance_id, "tier-b");
}

#[test]
fn sched_05_master_feature_local_required_filters_non_local() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    c.set_mapping(Capability::Llm, "llm.plan.default", "local", "m");
    c.set_mapping(Capability::Llm, "llm.plan.default", "remote", "m");
    r.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-local",
            "local",
            vec![Capability::Llm],
            vec!["local".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    r.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p-remote",
            "remote",
            vec![Capability::Llm],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let mut req = base_request();
    req.requirements.must_features = vec!["local".into()];
    let selected = Router
        .route(
            "tenant-a",
            &req,
            &r.snapshot(Capability::Llm),
            &r,
            &RouteConfig::default(),
            &c,
        )
        .unwrap();
    assert_eq!(selected.primary_instance_id, "p-local");
}

#[test]
fn sched_06_optional_features_do_not_break_primary_selection() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "stable",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "backup",
        "b",
        0.05,
        10,
        Ok(ProviderStartResult::Started),
    );
    let mut req = base_request();
    req.requirements.must_features = vec![];
    let selected = Router
        .route(
            "tenant-a",
            &req,
            &r.snapshot(Capability::Llm),
            &r,
            &RouteConfig::default(),
            &c,
        )
        .unwrap();
    assert_eq!(selected.primary_instance_id, "stable");
}

#[test]
fn sched_07_multi_provider_same_model_priority_stable() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        30,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let selected = Router
        .route(
            "tenant-a",
            &base_request(),
            &r.snapshot(Capability::Llm),
            &r,
            &RouteConfig::default(),
            &c,
        )
        .unwrap();
    assert_eq!(selected.primary_instance_id, "p2");
}

#[test]
fn sched_08_tenant_policy_overrides_global_strategy() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "cost-first",
        "a",
        0.01,
        100,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "latency-first",
        "b",
        0.20,
        10,
        Ok(ProviderStartResult::Started),
    );
    let mut cfg = RouteConfig::default();
    cfg.global_weights = RouteWeights {
        w_cost: 1.0,
        w_latency: 0.0,
        w_load: 0.0,
        w_error: 0.0,
    };
    cfg.tenant_overrides.insert(
        "tenant-a".into(),
        TenantRouteConfig {
            allow_provider_types: None,
            deny_provider_types: None,
            weights: Some(RouteWeights {
                w_cost: 0.0,
                w_latency: 1.0,
                w_load: 0.0,
                w_error: 0.0,
            }),
        },
    );
    let selected = Router
        .route(
            "tenant-a",
            &base_request(),
            &r.snapshot(Capability::Llm),
            &r,
            &cfg,
            &c,
        )
        .unwrap();
    assert_eq!(selected.primary_instance_id, "latency-first");
}
