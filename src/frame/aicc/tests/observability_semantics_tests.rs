mod common;

use aicc::{AIComputeCenter, CostEstimate, ModelCatalog, ProviderError, Registry};
use buckyos_api::{Capability, ResourceRef};
use common::*;
use std::sync::Arc;

#[tokio::test]
// 用例说明：
// - 验证场景：`obs_01_error_code_mapping_consistent` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：触发错误或日志链路，检查观测字段的映射与脱敏结果。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
    assert_eq!(
        extract_error_code(&sink.events_for(&bad_resp.task_id)).as_deref(),
        Some("bad_request"),
        "assert_eq failed in obs_01_error_code_mapping_consistent: expected left == right; check this scenario's routing/status/error-code branch."
    );

    let center_no_provider = {
        let mut c = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
        c.set_task_event_sink_factory(sink.clone());
        c
    };
    let no_provider_resp = center_no_provider
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(
        extract_error_code(&sink.events_for(&no_provider_resp.task_id)).as_deref(),
        Some("no_provider_available"),
        "assert_eq failed in obs_01_error_code_mapping_consistent: expected left == right; check this scenario's routing/status/error-code branch."
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`obs_02_log_redaction_no_prompt_or_base64` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：触发错误或日志链路，检查观测字段的映射与脱敏结果。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
    assert!(
        !msg.contains(secret),
        "assert failed in obs_02_log_redaction_no_prompt_or_base64: condition is false; check preconditions and expected branch outcome."
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`obs_01_provider_start_failed_code` 用例，覆盖函数名对应的业务路径。
// - 输入参数：按用例构造请求参数、路由配置和初始状态。
// - 处理流程：触发错误或日志链路，检查观测字段的映射与脱敏结果。
// - 预期输出：最终错误为 provider_start_failed。
async fn obs_01_provider_start_failed_code() {
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
        vec![Err(ProviderError::fatal("boom"))],
    )));
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(
        extract_error_code(&sink.events_for(&resp.task_id)).as_deref(),
        Some("provider_start_failed"),
        "assert_eq failed in obs_01_provider_start_failed_code: expected left == right; check this scenario's routing/status/error-code branch."
    );
}
