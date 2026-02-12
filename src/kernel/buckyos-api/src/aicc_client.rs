use crate::{AppDoc, AppType, SelectorType};
use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use ndn_lib::ObjId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;

pub const AICC_SERVICE_UNIQUE_ID: &str = "aicc";
pub const AICC_SERVICE_SERVICE_NAME: &str = "aicc";
pub const AICC_SERVICE_SERVICE_PORT: u16 = 4040;

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    LlmRouter,
    Text2Image,
    Text2Video,
    Text2Voice,
    Image2Text,
    Voice2Text,
    Video2Text,
}

pub type Feature = String;

pub mod features {
    pub const PLAN: &str = "plan";
    pub const TOOL_CALLING: &str = "tool_calling";
    pub const JSON_OUTPUT: &str = "json_output";
    pub const VISION: &str = "vision";
    pub const ASR: &str = "asr";
    pub const VIDEO_UNDERSTAND: &str = "video_understand";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceRef {
    Url {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_hint: Option<String>,
    },
    Base64 {
        mime: String,
        data_base64: String,
    },
    NamedObject {
        obj_id: ObjId,
    },
}

impl ResourceRef {
    pub fn url(url: String, mime_hint: Option<String>) -> Self {
        Self::Url { url, mime_hint }
    }

    pub fn base64(mime: String, data_base64: String) -> Self {
        Self::Base64 { mime, data_base64 }
    }

    pub fn named_object(obj_id: ObjId) -> Self {
        Self::NamedObject { obj_id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelSpec {
    pub alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_model_hint: Option<String>,
}

impl ModelSpec {
    pub fn new(alias: String, provider_model_hint: Option<String>) -> Self {
        Self {
            alias,
            provider_model_hint,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Requirements {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_features: Vec<Feature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

impl Requirements {
    pub fn new(
        must_features: Vec<Feature>,
        max_latency_ms: Option<u64>,
        max_cost_usd: Option<f64>,
        extra: Option<Value>,
    ) -> Self {
        Self {
            must_features,
            max_latency_ms,
            max_cost_usd,
            extra,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiMessage {
    pub role: String,
    pub content: String,
}

impl AiMessage {
    pub fn new(role: String, content: String) -> Self {
        Self { role, content }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<AiMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<ResourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_json: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
}

impl AiPayload {
    pub fn new(
        text: Option<String>,
        messages: Vec<AiMessage>,
        resources: Vec<ResourceRef>,
        input_json: Option<Value>,
        options: Option<Value>,
    ) -> Self {
        Self {
            text,
            messages,
            resources,
            input_json,
            options,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiCost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiArtifact {
    pub name: String,
    pub resource: ResourceRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiResponseSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<AiArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<AiCost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_task_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompleteRequest {
    pub capability: Capability,
    pub model: ModelSpec,
    pub requirements: Requirements,
    pub payload: AiPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<CompleteTaskOptions>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompleteTaskOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
}

impl CompleteRequest {
    pub fn new(
        capability: Capability,
        model: ModelSpec,
        requirements: Requirements,
        payload: AiPayload,
        idempotency_key: Option<String>,
    ) -> Self {
        Self {
            capability,
            model,
            requirements,
            payload,
            idempotency_key,
            task_options: None,
        }
    }

    pub fn with_task_options(mut self, task_options: Option<CompleteTaskOptions>) -> Self {
        self.task_options = task_options;
        self
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse CompleteRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompleteStatus {
    Succeeded,
    Running,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompleteResponse {
    pub task_id: String,
    pub status: CompleteStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AiResponseSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
}

impl CompleteResponse {
    pub fn new(
        task_id: String,
        status: CompleteStatus,
        result: Option<AiResponseSummary>,
        event_ref: Option<String>,
    ) -> Self {
        Self {
            task_id,
            status,
            result,
            event_ref,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelRequest {
    pub task_id: String,
}

impl CancelRequest {
    pub fn new(task_id: String) -> Self {
        Self { task_id }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse CancelRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelResponse {
    pub task_id: String,
    pub accepted: bool,
}

impl CancelResponse {
    pub fn new(task_id: String, accepted: bool) -> Self {
        Self { task_id, accepted }
    }
}

pub enum AiccClient {
    InProcess(Box<dyn AiccHandler>),
    KRPC(Box<kRPC>),
}

impl AiccClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_in_process(handler: Box<dyn AiccHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self::KRPC(krpc_client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await;
            }
        }
    }

    pub async fn complete(
        &self,
        request: CompleteRequest,
    ) -> std::result::Result<CompleteResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_complete(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize CompleteRequest: {}",
                        error
                    ))
                })?;
                let result = client.call("complete", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse complete response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn cancel(&self, task_id: &str) -> std::result::Result<CancelResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_cancel(task_id, ctx).await
            }
            Self::KRPC(client) => {
                let req = CancelRequest::new(task_id.to_string());
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!("Failed to serialize CancelRequest: {}", error))
                })?;
                let result = client.call("cancel", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse cancel response: {}",
                        error
                    ))
                })
            }
        }
    }
}

#[async_trait]
pub trait AiccHandler: Send + Sync {
    async fn handle_complete(
        &self,
        request: CompleteRequest,
        ctx: RPCContext,
    ) -> std::result::Result<CompleteResponse, RPCErrors>;

    async fn handle_cancel(
        &self,
        task_id: &str,
        ctx: RPCContext,
    ) -> std::result::Result<CancelResponse, RPCErrors>;
}

pub struct AiccServerHandler<T: AiccHandler>(pub T);

impl<T: AiccHandler> AiccServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: AiccHandler> RPCHandler for AiccServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            "complete" => {
                let complete_req = CompleteRequest::from_json(req.params)?;
                let result = self.0.handle_complete(complete_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "cancel" => {
                let cancel_req = CancelRequest::from_json(req.params)?;
                let result = self.0.handle_cancel(&cancel_req.task_id, ctx).await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

pub fn generate_aicc_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        AICC_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("AI Compute Center")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{Arc, Mutex};

    #[derive(Default, Debug)]
    struct MockCalls {
        complete: Option<CompleteRequest>,
        cancel_task_id: Option<String>,
    }

    #[derive(Clone)]
    struct MockAicc {
        calls: Arc<Mutex<MockCalls>>,
    }

    impl MockAicc {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(MockCalls::default())),
            }
        }
    }

    #[async_trait]
    impl AiccHandler for MockAicc {
        async fn handle_complete(
            &self,
            request: CompleteRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<CompleteResponse, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.complete = Some(request);
            Ok(CompleteResponse::new(
                "task-001".to_string(),
                CompleteStatus::Succeeded,
                Some(AiResponseSummary {
                    text: Some("mock result".to_string()),
                    json: Some(json!({"ok": true})),
                    artifacts: vec![],
                    usage: Some(AiUsage {
                        input_tokens: Some(4),
                        output_tokens: Some(8),
                        total_tokens: Some(12),
                    }),
                    cost: Some(AiCost {
                        amount: 0.001,
                        currency: "USD".to_string(),
                    }),
                    finish_reason: Some("stop".to_string()),
                    provider_task_ref: Some("provider-task-001".to_string()),
                    extra: None,
                }),
                Some("task://task-001/events".to_string()),
            ))
        }

        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<CancelResponse, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.cancel_task_id = Some(task_id.to_string());
            Ok(CancelResponse::new(task_id.to_string(), true))
        }
    }

    fn sample_complete_request() -> CompleteRequest {
        CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new("llm.plan.default".to_string(), None),
            Requirements::new(vec![features::PLAN.to_string()], Some(3000), None, None),
            AiPayload::new(
                Some("write a release note".to_string()),
                vec![AiMessage::new(
                    "user".to_string(),
                    "summarize this commit".to_string(),
                )],
                vec![
                    ResourceRef::url(
                        "cyfs://example/object/1".to_string(),
                        Some("text/plain".to_string()),
                    ),
                    ResourceRef::named_object(ObjId::new("chunk:123456").unwrap()),
                ],
                None,
                Some(json!({"temperature": 0.3})),
            ),
            Some("idem-1".to_string()),
        )
    }

    #[test]
    fn test_generate_aicc_service_doc() {
        let doc = generate_aicc_service_doc();
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }

    #[tokio::test]
    async fn test_in_process_client_with_mock() {
        let mock = MockAicc::new();
        let calls = mock.calls.clone();
        let client = AiccClient::new_in_process(Box::new(mock));

        let request = sample_complete_request();
        let complete_result = client.complete(request.clone()).await.unwrap();
        assert_eq!(complete_result.task_id, "task-001");
        assert_eq!(complete_result.status, CompleteStatus::Succeeded);
        assert_eq!(
            complete_result
                .result
                .as_ref()
                .and_then(|summary| summary.text.as_ref())
                .map(|text| text.as_str()),
            Some("mock result")
        );

        let cancel_result = client.cancel("task-001").await.unwrap();
        assert_eq!(cancel_result.task_id, "task-001");
        assert!(cancel_result.accepted);

        let calls = calls.lock().unwrap();
        assert_eq!(calls.complete, Some(request));
        assert_eq!(calls.cancel_task_id.as_deref(), Some("task-001"));
    }

    #[tokio::test]
    async fn test_rpc_handler_adapter_with_mock() {
        let mock = MockAicc::new();
        let calls = mock.calls.clone();
        let rpc_handler = AiccServerHandler::new(mock);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        let request = sample_complete_request();
        let complete_req = RPCRequest {
            method: "complete".to_string(),
            params: serde_json::to_value(&request).unwrap(),
            seq: 9,
            token: None,
            trace_id: None,
        };
        let complete_resp = rpc_handler.handle_rpc_call(complete_req, ip).await.unwrap();
        match complete_resp.result {
            RPCResult::Success(value) => {
                let complete_result: CompleteResponse = serde_json::from_value(value).unwrap();
                assert_eq!(complete_result.task_id, "task-001");
                assert_eq!(complete_result.status, CompleteStatus::Succeeded);
            }
            _ => panic!("Expected success response"),
        }

        let cancel_req = RPCRequest {
            method: "cancel".to_string(),
            params: json!({"task_id": "task-001"}),
            seq: 10,
            token: None,
            trace_id: None,
        };
        let cancel_resp = rpc_handler.handle_rpc_call(cancel_req, ip).await.unwrap();
        match cancel_resp.result {
            RPCResult::Success(value) => {
                let cancel_result: CancelResponse = serde_json::from_value(value).unwrap();
                assert_eq!(cancel_result.task_id, "task-001");
                assert!(cancel_result.accepted);
            }
            _ => panic!("Expected success response"),
        }

        let calls = calls.lock().unwrap();
        assert_eq!(calls.complete, Some(request));
        assert_eq!(calls.cancel_task_id.as_deref(), Some("task-001"));
    }
}
