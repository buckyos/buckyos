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

#[tokio::test]
async fn proto_sec_01_no_base64_in_logs() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let secret = openai_b64(&[9, 8, 7, 6, 5, 4]);
    setup_route_provider(&registry, &catalog, "p1", "provider-a", "m", 0.01, 10);
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p2",
            "provider-a",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: Some("ok".into()),
            tool_calls: vec![],
            artifacts: vec![buckyos_api::AiArtifact {
                name: "artifact".into(),
                resource: ResourceRef::Base64 {
                    mime: "image/png".into(),
                    data_base64: secret.clone(),
                },
                mime: Some("image/png".into()),
                metadata: None,
            }],
            usage: None,
            cost: None,
            finish_reason: Some("stop".into()),
            provider_task_ref: None,
            extra: None,
        }))],
    )));
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let events = sink.events_for(&resp.task_id);
    let encoded_events = serde_json::to_string(&events).unwrap();
    assert!(!encoded_events.contains(secret.as_str()));
}

#[tokio::test]
async fn proto_sec_02_no_prompt_in_logs() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p1", "provider-a", "m", 0.01, 10);
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    let secret_prompt = "prompt-secret-for-redaction-check";
    let mut req = base_request();
    req.payload.text = Some(secret_prompt.to_string());
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let events = sink.events_for(&resp.task_id);
    let encoded_events = serde_json::to_string(&events).unwrap();
    assert!(!encoded_events.contains(secret_prompt));
}

#[tokio::test]
async fn proto_res_01_named_object_passthrough_preserved() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p1", "provider-a", "m", 0.01, 10);
    let center = center_with_taskmgr(registry, catalog);
    let req = request_with_resource(
        serde_json::from_value(serde_json::json!({
            "kind": "named_object",
            "obj_id": "chunk:123456"
        }))
        .unwrap(),
    );
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(resp.status, CompleteStatus::Running);
    let taskmgr = center.task_manager_client().unwrap();
    let tasks = taskmgr
        .list_tasks(None::<TaskFilter>, None, None)
        .await
        .unwrap();
    let task = tasks
        .into_iter()
        .find(|t| {
            t.data
                .pointer("/aicc/external_task_id")
                .and_then(|v| v.as_str())
                == Some(resp.task_id.as_str())
        })
        .unwrap();
    assert_eq!(
        task.data
            .pointer("/aicc/request/payload/resources/0/kind")
            .and_then(|v| v.as_str()),
        Some("named_object")
    );
    assert_eq!(
        task.data
            .pointer("/aicc/request/payload/resources/0/obj_id")
            .and_then(|v| v.as_str()),
        Some("chunk:123456")
    );
}

#[tokio::test]
async fn proto_res_02_cyfs_url_scheme_policy_allowed() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    center.set_url_scheme_allowlist(string_set(&["http", "https", "cyfs"]));
    let req = request_with_resource(ResourceRef::Url {
        url: "cyfs://example/object/1".to_string(),
        mime_hint: Some("text/plain".to_string()),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_ne!(
        extract_error_code(&sink.events_for(&resp.task_id)).as_deref(),
        Some("resource_invalid")
    );
}

#[tokio::test]
async fn proto_res_03_cyfs_url_scheme_policy_rejected() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    center.set_url_scheme_allowlist(string_set(&["http", "https"]));
    let req = request_with_resource(ResourceRef::Url {
        url: "cyfs://example/object/1".to_string(),
        mime_hint: Some("text/plain".to_string()),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(
        extract_error_code(&sink.events_for(&resp.task_id)).as_deref(),
        Some("resource_invalid")
    );
}

#[tokio::test]
async fn proto_res_04_named_object_requires_resolver_when_provider_needs_bytes() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    center.set_resource_resolver(Arc::new(FailingResolver {
        message: "resolver required for named object".to_string(),
    }));
    let req = request_with_resource(ResourceRef::NamedObject {
        obj_id: serde_json::from_value(serde_json::json!("chunk:123456")).unwrap(),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(
        extract_error_code(&sink.events_for(&resp.task_id)).as_deref(),
        Some("resource_invalid")
    );
}

#[tokio::test]
async fn proto_res_05_equivalent_resource_semantics_base64_url_named_object() {
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());
    let req_base64 = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    });
    let req_url = request_with_resource(ResourceRef::Url {
        url: "https://example.com/1.png".to_string(),
        mime_hint: Some("image/png".to_string()),
    });
    let req_named = request_with_resource(ResourceRef::NamedObject {
        obj_id: serde_json::from_value(serde_json::json!("chunk:123456")).unwrap(),
    });
    let r1 = center
        .complete(req_base64, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let r2 = center
        .complete(req_url, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let r3 = center
        .complete(req_named, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let c1 = extract_error_code(&sink.events_for(&r1.task_id));
    let c2 = extract_error_code(&sink.events_for(&r2.task_id));
    let c3 = extract_error_code(&sink.events_for(&r3.task_id));
    assert_eq!(c1.as_deref(), Some("no_provider_available"));
    assert_eq!(c2.as_deref(), Some("no_provider_available"));
    assert_eq!(c3.as_deref(), Some("no_provider_available"));
}

#[tokio::test]
async fn proto_res_06_base64_to_url_translation_for_url_only_provider() {
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    center.set_task_event_sink_factory(sink.clone());
    center.set_resource_resolver(Arc::new(FailingResolver {
        message: "url-only provider requires translated URL".to_string(),
    }));
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(
        extract_error_code(&sink.events_for(&resp.task_id)).as_deref(),
        Some("resource_invalid")
    );
}

#[tokio::test]
async fn proto_res_07_provider_base64_unsupported_error_classified() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m");
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p1",
            "provider-a",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Err(ProviderError::fatal(
            "provider does not support base64 resources",
        ))],
    )));
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    let req = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(&[1, 2, 3]),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    assert_eq!(
        extract_error_code(&sink.events_for(&resp.task_id)).as_deref(),
        Some("provider_start_failed")
    );
}

#[tokio::test]
async fn proto_res_08_named_object_and_url_mixed_order_stable() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p1", "provider-a", "m", 0.01, 10);
    let center = center_with_taskmgr(registry, catalog);
    let req = {
        let mut base = base_request();
        base.payload.resources = vec![
            ResourceRef::NamedObject {
                obj_id: serde_json::from_value(serde_json::json!("chunk:123456")).unwrap(),
            },
            ResourceRef::Url {
                url: "https://example.com/1.png".to_string(),
                mime_hint: Some("image/png".to_string()),
            },
        ];
        base
    };
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let taskmgr = center.task_manager_client().unwrap();
    let tasks = taskmgr
        .list_tasks(None::<TaskFilter>, None, None)
        .await
        .unwrap();
    let task = tasks
        .into_iter()
        .find(|t| {
            t.data
                .pointer("/aicc/external_task_id")
                .and_then(|v| v.as_str())
                == Some(resp.task_id.as_str())
        })
        .unwrap();
    assert_eq!(
        task.data
            .pointer("/aicc/request/payload/resources/0/kind")
            .and_then(|v| v.as_str()),
        Some("named_object")
    );
    assert_eq!(
        task.data
            .pointer("/aicc/request/payload/resources/1/kind")
            .and_then(|v| v.as_str()),
        Some("url")
    );
}

#[tokio::test]
async fn proto_res_09_mime_hint_consistency_after_translation() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    setup_route_provider(&registry, &catalog, "p1", "provider-a", "m", 0.01, 10);
    let center = center_with_taskmgr(registry, catalog);
    let req = request_with_resource(ResourceRef::Url {
        url: "https://example.com/1.png".to_string(),
        mime_hint: Some("image/png".to_string()),
    });
    let resp = center
        .complete(req, rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let taskmgr = center.task_manager_client().unwrap();
    let tasks = taskmgr
        .list_tasks(None::<TaskFilter>, None, None)
        .await
        .unwrap();
    let task = tasks
        .into_iter()
        .find(|t| {
            t.data
                .pointer("/aicc/external_task_id")
                .and_then(|v| v.as_str())
                == Some(resp.task_id.as_str())
        })
        .unwrap();
    assert_eq!(
        task.data
            .pointer("/aicc/request/payload/resources/0/mime_hint")
            .and_then(|v| v.as_str()),
        Some("image/png")
    );
}

#[tokio::test]
async fn proto_res_10_no_sensitive_resource_literal_in_provider_logs() {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", "provider-a", "m");
    let secret = openai_b64(&[3, 3, 3, 3, 9, 9]);
    registry.add_provider(Arc::new(MockProvider::new(
        mock_instance(
            "p1",
            "provider-a",
            vec![Capability::LlmRouter],
            vec!["plan".into()],
        ),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: Some("ok".into()),
            tool_calls: vec![],
            artifacts: vec![buckyos_api::AiArtifact {
                name: "redact".into(),
                resource: ResourceRef::Base64 {
                    mime: "image/png".into(),
                    data_base64: secret.clone(),
                },
                mime: Some("image/png".into()),
                metadata: Some(serde_json::json!({"signed_url":"https://example.com?p=secret"})),
            }],
            usage: None,
            cost: None,
            finish_reason: Some("stop".into()),
            provider_task_ref: None,
            extra: None,
        }))],
    )));
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(registry, catalog);
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(base_request(), rpc_ctx_with_tenant(None))
        .await
        .unwrap();
    let event_json = serde_json::to_string(&sink.events_for(&resp.task_id)).unwrap();
    assert!(!event_json.contains(secret.as_str()));
}
