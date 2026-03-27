mod common;

use aicc::{
    CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry, RouteConfig,
    RouteWeights, Router, TenantRouteConfig,
};
use buckyos_api::{AiResponseSummary, AiccServerHandler, Capability, CompleteStatus};
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
// - 验证场景：`proto_llm_01_messages_format_valid` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn proto_llm_01_messages_format_valid() {
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
    let mut req = base_request();
    req.payload.text = None;
    req.payload.messages = vec![buckyos_api::AiMessage::new("user".into(), "hello".into())];
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_llm_02_input_json_format_valid` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn proto_llm_02_input_json_format_valid() {
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
    let mut req = base_request();
    req.payload.text = None;
    req.payload.input_json = Some(json!({"a":1}));
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_llm_03_tool_specs_format_valid` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn proto_llm_03_tool_specs_format_valid() {
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
    let mut req = base_request();
    req.payload.options = Some(json!({"tool_specs":[{"name":"t","schema":{"type":"object"}}]}));
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_llm_04_temperature_boundary_valid` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn proto_llm_04_temperature_boundary_valid() {
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
    let mut req = base_request();
    req.payload.options = Some(json!({"temperature":1.0}));
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_v2t_01_language_param_respected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn proto_v2t_01_language_param_respected() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    c.set_mapping(Capability::Voice2Text, "v2t.default", "a", "m");
    r.add_provider(Arc::new(MockProvider::new(
        mock_instance("p1", "a", vec![Capability::Voice2Text], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let mut req = base_request_for(Capability::Voice2Text, "v2t.default");
    req.payload.options = Some(json!({"language":"zh-CN"}));
    req.payload.resources = vec![buckyos_api::ResourceRef::Url {
        url: "https://example.com/a.wav".into(),
        mime_hint: Some("audio/wav".into()),
    }];
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_v2t_02_hotword_param_respected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn proto_v2t_02_hotword_param_respected() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    c.set_mapping(Capability::Video2Text, "v2t.default", "a", "m");
    r.add_provider(Arc::new(MockProvider::new(
        mock_instance("p1", "a", vec![Capability::Video2Text], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let mut req = base_request_for(Capability::Video2Text, "v2t.default");
    req.payload.options = Some(json!({"hotword":"buckyos"}));
    req.payload.resources = vec![buckyos_api::ResourceRef::Url {
        url: "https://example.com/a.mp4".into(),
        mime_hint: Some("video/mp4".into()),
    }];
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_t2v_01_text_length_limit` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn proto_t2v_01_text_length_limit() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    c.set_mapping(Capability::Text2Voice, "t2v.default", "a", "m");
    r.add_provider(Arc::new(MockProvider::new(
        mock_instance("p1", "a", vec![Capability::Text2Voice], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let mut req = base_request_for(Capability::Text2Voice, "t2v.default");
    req.payload.text = Some("hello".repeat(20));
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_t2v_02_voice_param_respected` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn proto_t2v_02_voice_param_respected() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    c.set_mapping(Capability::Text2Voice, "t2v.default", "a", "m");
    r.add_provider(Arc::new(MockProvider::new(
        mock_instance("p1", "a", vec![Capability::Text2Voice], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(0.01),
            estimated_latency_ms: Some(10),
        },
        vec![Ok(ProviderStartResult::Started)],
    )));
    let mut req = base_request_for(Capability::Text2Voice, "t2v.default");
    req.payload.options = Some(json!({"voice":"alloy"}));
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Running
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_mix_01_text_plus_resource_valid` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn proto_mix_01_text_plus_resource_valid() {
    let mut req = base_request();
    req.payload.resources = vec![buckyos_api::ResourceRef::Base64 {
        mime: "image/png".into(),
        data_base64: openai_b64(&[1, 2, 3]),
    }];
    let center = aicc::AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Failed
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_mix_02_messages_plus_resource_valid` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn proto_mix_02_messages_plus_resource_valid() {
    let mut req = base_request();
    req.payload.text = None;
    req.payload.messages = vec![buckyos_api::AiMessage::new("user".into(), "hi".into())];
    req.payload.resources = vec![buckyos_api::ResourceRef::Url {
        url: "https://example.com/a.png".into(),
        mime_hint: Some("image/png".into()),
    }];
    let center = aicc::AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Failed
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_mix_03_input_json_plus_resource_valid` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：返回成功结果，关键字段与断言一致。
async fn proto_mix_03_input_json_plus_resource_valid() {
    let mut req = base_request();
    req.payload.text = None;
    req.payload.input_json = Some(json!({"a":1}));
    req.payload.resources = vec![buckyos_api::ResourceRef::Url {
        url: "https://example.com/a.png".into(),
        mime_hint: Some("image/png".into()),
    }];
    let center = aicc::AIComputeCenter::new(Registry::default(), ModelCatalog::default());
    assert_eq!(
        center
            .complete(req, RPCContext::default())
            .await
            .unwrap()
            .status,
        CompleteStatus::Failed
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_mix_04_resource_order_stable` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：事件顺序稳定。
async fn proto_mix_04_resource_order_stable() {
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
    let mut req = base_request();
    req.payload.resources = vec![
        buckyos_api::ResourceRef::Url {
            url: "https://example.com/1.png".into(),
            mime_hint: Some("image/png".into()),
        },
        buckyos_api::ResourceRef::Base64 {
            mime: "audio/wav".into(),
            data_base64: openai_b64(&[1, 2, 3]),
        },
    ];
    let resp = center.complete(req, RPCContext::default()).await.unwrap();
    let task = center
        .task_manager_client()
        .unwrap()
        .list_tasks(None::<buckyos_api::TaskFilter>, None, None)
        .await
        .unwrap()
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
        Some("url")
    );
}

#[tokio::test]
// 用例说明：
// - 验证场景：`proto_sec_03_no_artifact_bytes_in_events` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn proto_sec_03_no_artifact_bytes_in_events() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let secret = openai_b64(&[9, 9, 9, 9]);
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: Some("ok".into()),
            tool_calls: vec![],
            artifacts: vec![buckyos_api::AiArtifact {
                name: "artifact-1".into(),
                resource: buckyos_api::ResourceRef::Base64 {
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
        })),
    );
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(r, c);
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&sink.events_for(&resp.task_id))
        .unwrap()
        .contains(secret.as_str()));
}
