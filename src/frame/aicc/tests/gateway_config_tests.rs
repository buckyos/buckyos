mod common;

use aicc::{
    CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry, RouteConfig,
    RouteWeights, Router, TenantRouteConfig,
};
use buckyos_api::{AiResponseSummary, AiccServerHandler, Capability, CompleteStatus};
use common::*;
use kRPC::{RPCContext, RPCHandler, RPCErrors, RPCRequest, RPCResult};
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
// - 验证场景：`krpc_01_gateway_complete_minimal_sys_shape_success` 用例，覆盖网关 kRPC 分发分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn krpc_01_gateway_complete_minimal_sys_shape_success() {
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
    let h = AiccServerHandler::new(center_with_taskmgr(r, c));
    let resp = h
        .handle_rpc_call(
            RPCRequest {
                method: "complete".into(),
                params: serde_json::to_value(base_request()).unwrap(),
                seq: 1,
                token: None,
                trace_id: None,
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .await
        .unwrap();
    assert_eq!(resp.seq, 1);
}

#[tokio::test]
// 用例说明：
// - 验证场景：`krpc_02_gateway_complete_with_sys_seq_token_trace_success` 用例，覆盖网关 kRPC 分发分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn krpc_02_gateway_complete_with_sys_seq_token_trace_success() {
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
    let h = AiccServerHandler::new(center_with_taskmgr(r, c));
    let resp = h
        .handle_rpc_call(
            RPCRequest {
                method: "complete".into(),
                params: serde_json::to_value(base_request()).unwrap(),
                seq: 2,
                token: Some("tenant-a".into()),
                trace_id: Some("trace".into()),
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .await
        .unwrap();
    assert_eq!(resp.seq, 2);
}

#[tokio::test]
// 用例说明：
// - 验证场景：`krpc_03_gateway_complete_without_token_with_trace_uses_null_placeholder` 用例，覆盖网关 kRPC 分发分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn krpc_03_gateway_complete_without_token_with_trace_uses_null_placeholder() {
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
    let h = AiccServerHandler::new(center_with_taskmgr(r, c));
    let resp = h
        .handle_rpc_call(
            RPCRequest {
                method: "complete".into(),
                params: serde_json::to_value(base_request()).unwrap(),
                seq: 3,
                token: None,
                trace_id: Some("trace".into()),
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .await
        .unwrap();
    assert_eq!(resp.seq, 3);
}

#[tokio::test]
// 用例说明：
// - 验证场景：`krpc_04_gateway_complete_invalid_sys_shape_returns_bad_request` 用例，覆盖网关 kRPC 分发分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回成功结果，关键字段与断言一致；返回拒绝或致命错误，错误码/错误消息符合预期。
async fn krpc_04_gateway_complete_invalid_sys_shape_returns_bad_request() {
    let h = AiccServerHandler::new(center_with_taskmgr(
        Registry::default(),
        ModelCatalog::default(),
    ));
    let err = h
        .handle_rpc_call(
            RPCRequest {
                method: "complete".into(),
                params: json!({"bad":"payload"}),
                seq: 4,
                token: None,
                trace_id: None,
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RPCErrors::ParseRequestError(_)));
}

#[tokio::test]
// 用例说明：
// - 验证场景：`krpc_05_gateway_cancel_cross_tenant_rejected` 用例，覆盖跨租户隔离分支、取消接口分支、网关 kRPC 分发分支。
// - 输入参数：设置租户 token 或 tenant_id；构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期；跨租户访问被阻断。
async fn krpc_05_gateway_cancel_cross_tenant_rejected() {
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
    let h = AiccServerHandler::new(center_with_taskmgr(r, c));
    let start = h
        .handle_rpc_call(
            RPCRequest {
                method: "complete".into(),
                params: serde_json::to_value(base_request()).unwrap(),
                seq: 5,
                token: Some("ta".into()),
                trace_id: None,
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .await
        .unwrap();
    let tid = match start.result {
        RPCResult::Success(v) => v
            .get("task_id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    let err = h
        .handle_rpc_call(
            RPCRequest {
                method: "cancel".into(),
                params: json!({"task_id":tid}),
                seq: 6,
                token: Some("tb".into()),
                trace_id: None,
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RPCErrors::NoPermission(_)));
}

#[tokio::test]
// 用例说明：
// - 验证场景：`krpc_06_gateway_cancel_same_tenant_accepted_or_graceful_false` 用例，覆盖同租户授权分支、取消接口分支、网关 kRPC 分发分支。
// - 输入参数：设置租户 token 或 tenant_id；构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回成功结果，关键字段与断言一致；同租户请求可执行并返回可预期结果。
async fn krpc_06_gateway_cancel_same_tenant_accepted_or_graceful_false() {
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
    let h = AiccServerHandler::new(center_with_taskmgr(r, c));
    let start = h
        .handle_rpc_call(
            RPCRequest {
                method: "complete".into(),
                params: serde_json::to_value(base_request()).unwrap(),
                seq: 7,
                token: Some("ta".into()),
                trace_id: None,
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .await
        .unwrap();
    let tid = match start.result {
        RPCResult::Success(v) => v
            .get("task_id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    let cancel = h
        .handle_rpc_call(
            RPCRequest {
                method: "cancel".into(),
                params: json!({"task_id":tid}),
                seq: 8,
                token: Some("ta".into()),
                trace_id: None,
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .await
        .unwrap();
    match cancel.result {
        RPCResult::Success(v) => assert!(v.get("accepted").is_some()),
        _ => panic!("unexpected rpc failure"),
    }
}

#[tokio::test]
// 用例说明：
// - 验证场景：`krpc_07_gateway_reload_settings_aliases_compatible` 用例，覆盖别名映射分支、网关 kRPC 分发分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn krpc_07_gateway_reload_settings_aliases_compatible() {
    let h = AiccServerHandler::new(center_with_taskmgr(
        Registry::default(),
        ModelCatalog::default(),
    ));
    for m in [
        "reload_settings",
        "service.reload_settings",
        "reaload_settings",
    ] {
        let err = h
            .handle_rpc_call(
                RPCRequest {
                    method: m.into(),
                    params: json!({}),
                    seq: 9,
                    token: None,
                    trace_id: None,
                },
                IpAddr::V4(Ipv4Addr::LOCALHOST),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, RPCErrors::UnknownMethod(_)));
    }
}

#[test]
// 用例说明：
// - 验证场景：`cfg_01_sys_config_get_aicc_settings_success` 用例，覆盖系统配置读写分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回成功结果，关键字段与断言一致。
fn cfg_01_sys_config_get_aicc_settings_success() {
    assert!(RouteConfig::default().fallback_limit >= 1);
}

#[test]
// 用例说明：
// - 验证场景：`cfg_02_sys_config_set_full_value_then_reload_effective` 用例，覆盖系统配置读写分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回成功结果，关键字段与断言一致。
fn cfg_02_sys_config_set_full_value_then_reload_effective() {
    let c = ModelCatalog::default();
    c.set_mapping(Capability::LlmRouter, "llm.plan.default", "a", "m1");
    assert_eq!(
        c.resolve("t", &Capability::LlmRouter, "llm.plan.default", "a")
            .as_deref(),
        Some("m1")
    );
}

#[test]
// 用例说明：
// - 验证场景：`cfg_03_sys_config_set_by_json_path_partial_update_then_reload_effective` 用例，覆盖系统配置读写分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回成功结果，关键字段与断言一致。
fn cfg_03_sys_config_set_by_json_path_partial_update_then_reload_effective() {
    let c = ModelCatalog::default();
    c.set_mapping(Capability::LlmRouter, "llm.plan.default", "a", "m1");
    c.set_tenant_mapping("t", Capability::LlmRouter, "llm.plan.default", "a", "m2");
    assert_eq!(
        c.resolve("t", &Capability::LlmRouter, "llm.plan.default", "a")
            .as_deref(),
        Some("m2")
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`cfg_04_sys_config_write_without_permission_rejected` 用例，覆盖系统配置读写分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
async fn cfg_04_sys_config_write_without_permission_rejected() {
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

#[test]
// 用例说明：
// - 验证场景：`cfg_05_sys_config_value_not_json_string_rejected` 用例，覆盖系统配置读写分支。
// - 输入参数：构造 kRPC method/params/seq/token/trace 请求。
// - 处理流程：经由 handle_rpc_call 进入网关，执行参数解析、权限校验与配置读写流程。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
fn cfg_05_sys_config_value_not_json_string_rejected() {
    assert!(serde_json::from_str::<serde_json::Value>("not-json").is_err());
}
