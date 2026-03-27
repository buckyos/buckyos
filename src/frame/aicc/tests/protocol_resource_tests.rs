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

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_b64_06_invalid_mime_rejected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致；返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`proto_b64_07_size_limit_exceeded_rejected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`proto_b64_08_malformed_base64_rejected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`proto_url_03_missing_scheme_rejected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`proto_url_04_empty_url_rejected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`proto_url_05_invalid_url_format_rejected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致；返回拒绝或致命错误，错误码/错误消息符合预期。
async fn proto_url_05_invalid_url_format_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Url {
        url: "https://exa mple.com/abc.png".to_string(),
        mime_hint: None,
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(
        extract_error_code(&sink.events_for(&resp.task_id)).as_deref(),
        Some("resource_invalid"),
        "assert_eq failed in proto_url_05_invalid_url_format_rejected: expected left == right; check this scenario's routing/status/error-code branch."
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_sec_04_idempotency_key_preserved` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：需保留字段在链路中不被改写。
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
// 用例说明：
// - 验证场景：`proto_url_06_resource_unreachable_simulated` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
// 用例说明：
// - 验证场景：`proto_url_01_https_valid` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
// 用例说明：
// - 验证场景：`proto_url_02_http_allowed` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
// 用例说明：
// - 验证场景：`proto_b64_01_image_valid_png` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
// 用例说明：
// - 验证场景：`proto_b64_02_image_valid_jpeg` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
// 用例说明：
// - 验证场景：`proto_b64_03_audio_valid_wav` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
// 用例说明：
// - 验证场景：`proto_b64_04_audio_valid_mp3` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
// 用例说明：
// - 验证场景：`proto_b64_05_video_valid_mp4` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
