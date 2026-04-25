use crate::{AppDoc, AppType, SelectorType};
use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use ndn_lib::ObjId;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;

pub const AICC_SERVICE_UNIQUE_ID: &str = "aicc";
pub const AICC_SERVICE_SERVICE_NAME: &str = "aicc";
pub const AICC_SERVICE_SERVICE_PORT: u16 = 4040;

pub mod ai_methods {
    pub const LLM_CHAT: &str = "llm.chat";
    pub const LLM_COMPLETION: &str = "llm.completion";
    pub const EMBEDDING_TEXT: &str = "embedding.text";
    pub const EMBEDDING_MULTIMODAL: &str = "embedding.multimodal";
    pub const RERANK: &str = "rerank";
    pub const IMAGE_TXT2IMG: &str = "image.txt2img";
    pub const IMAGE_IMG2IMG: &str = "image.img2img";
    pub const IMAGE_INPAINT: &str = "image.inpaint";
    pub const IMAGE_UPSCALE: &str = "image.upscale";
    pub const IMAGE_BG_REMOVE: &str = "image.bg_remove";
    pub const VISION_OCR: &str = "vision.ocr";
    pub const VISION_CAPTION: &str = "vision.caption";
    pub const VISION_DETECT: &str = "vision.detect";
    pub const VISION_SEGMENT: &str = "vision.segment";
    pub const AUDIO_TTS: &str = "audio.tts";
    pub const AUDIO_ASR: &str = "audio.asr";
    pub const AUDIO_MUSIC: &str = "audio.music";
    pub const AUDIO_ENHANCE: &str = "audio.enhance";
    pub const VIDEO_TXT2VIDEO: &str = "video.txt2video";
    pub const VIDEO_IMG2VIDEO: &str = "video.img2video";
    pub const VIDEO_VIDEO2VIDEO: &str = "video.video2video";
    pub const VIDEO_EXTEND: &str = "video.extend";
    pub const VIDEO_UPSCALE: &str = "video.upscale";
    pub const AGENT_COMPUTER_USE: &str = "agent.computer_use";

    pub const CANCEL: &str = "cancel";
    pub const RELOAD_SETTINGS: &str = "reload_settings";
    pub const SERVICE_RELOAD_SETTINGS: &str = "service.reload_settings";
    pub const QUOTA_QUERY: &str = "quota.query";
    pub const PROVIDER_LIST: &str = "provider.list";
    pub const PROVIDER_HEALTH: &str = "provider.health";

    pub fn is_ai_method(method: &str) -> bool {
        matches!(
            method,
            LLM_CHAT
                | LLM_COMPLETION
                | EMBEDDING_TEXT
                | EMBEDDING_MULTIMODAL
                | RERANK
                | IMAGE_TXT2IMG
                | IMAGE_IMG2IMG
                | IMAGE_INPAINT
                | IMAGE_UPSCALE
                | IMAGE_BG_REMOVE
                | VISION_OCR
                | VISION_CAPTION
                | VISION_DETECT
                | VISION_SEGMENT
                | AUDIO_TTS
                | AUDIO_ASR
                | AUDIO_MUSIC
                | AUDIO_ENHANCE
                | VIDEO_TXT2VIDEO
                | VIDEO_IMG2VIDEO
                | VIDEO_VIDEO2VIDEO
                | VIDEO_EXTEND
                | VIDEO_UPSCALE
                | AGENT_COMPUTER_USE
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Llm,
    Embedding,
    Rerank,
    Image,
    Vision,
    Audio,
    Video,
    Agent,
}

pub type Feature = String;

pub mod features {
    pub const PLAN: &str = "plan";
    pub const TOOL_CALLING: &str = "tool_calling";
    pub const JSON_OUTPUT: &str = "json_output";
    pub const WEB_SEARCH: &str = "web_search";
    pub const VISION: &str = "vision";
    pub const ASR: &str = "asr";
    pub const VIDEO_UNDERSTAND: &str = "video_understand";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RespFormat {
    #[default]
    #[serde(alias = "Text")]
    Text,
    #[serde(alias = "Json", alias = "JSON")]
    Json,
}

fn is_default_resp_format(resp_format: &RespFormat) -> bool {
    matches!(resp_format, RespFormat::Text)
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
    #[serde(default, skip_serializing_if = "is_default_resp_format")]
    pub resp_format: RespFormat,
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
            resp_format: RespFormat::default(),
            extra,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RoutePolicyProfile {
    Cheap,
    Fast,
    #[default]
    Balanced,
    Quality,
}

fn is_default_route_policy_profile(profile: &RoutePolicyProfile) -> bool {
    matches!(profile, RoutePolicyProfile::Balanced)
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutePolicy {
    #[serde(default, skip_serializing_if = "is_default_route_policy_profile")]
    pub profile: RoutePolicyProfile,
    #[serde(default = "default_allow_fallback")]
    pub allow_fallback: bool,
    #[serde(default = "default_runtime_failover")]
    pub runtime_failover: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub explain: bool,
}

fn default_allow_fallback() -> bool {
    true
}

fn default_runtime_failover() -> bool {
    true
}

impl Default for RoutePolicy {
    fn default() -> Self {
        Self {
            profile: RoutePolicyProfile::Balanced,
            allow_fallback: true,
            runtime_failover: true,
            explain: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AiToolSpec {
    pub name: String,
    pub description: String,
    pub args_schema: HashMap<String, Value>,
    pub output_schema: Value,
}

pub fn value_to_object_map(value: Value) -> HashMap<String, Value> {
    match value {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
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

#[derive(Debug, Clone, PartialEq)]
pub struct AiPayload {
    pub text: Option<String>,
    pub messages: Vec<AiMessage>,
    pub tool_specs: Vec<AiToolSpec>,
    pub resources: Vec<ResourceRef>,
    pub input_json: Option<Value>,
    pub options: Option<Value>,
}

impl AiPayload {
    pub fn new(
        text: Option<String>,
        messages: Vec<AiMessage>,
        tool_specs: Vec<AiToolSpec>,
        resources: Vec<ResourceRef>,
        input_json: Option<Value>,
        options: Option<Value>,
    ) -> Self {
        Self {
            text,
            messages,
            tool_specs,
            resources,
            input_json,
            options,
        }
    }

    fn protocol_input_json(&self) -> Value {
        let mut input_json = match self.input_json.clone() {
            Some(Value::Object(map)) => map,
            Some(value) => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_string(), value);
                map
            }
            None => serde_json::Map::new(),
        };

        if let Some(text) = self.text.as_ref() {
            input_json
                .entry("text".to_string())
                .or_insert_with(|| Value::String(text.clone()));
        }
        if !self.messages.is_empty() && !input_json.contains_key("messages") {
            if let Ok(value) = serde_json::to_value(&self.messages) {
                input_json.insert("messages".to_string(), value);
            }
        }
        if !self.tool_specs.is_empty() && !input_json.contains_key("tool_specs") {
            if let Ok(value) = serde_json::to_value(&self.tool_specs) {
                input_json.insert("tool_specs".to_string(), value);
            }
        }

        Value::Object(input_json)
    }
}

impl Default for AiPayload {
    fn default() -> Self {
        Self {
            text: None,
            messages: vec![],
            tool_specs: vec![],
            resources: vec![],
            input_json: Some(json!({})),
            options: Some(json!({})),
        }
    }
}

impl Serialize for AiPayload {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let options = self.options.clone().unwrap_or_else(|| json!({}));
        let mut state = serializer.serialize_struct("AiPayload", 3)?;
        state.serialize_field("input_json", &self.protocol_input_json())?;
        state.serialize_field("resources", &self.resources)?;
        state.serialize_field("options", &options)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for AiPayload {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AiPayloadHelper {
            #[serde(default)]
            input_json: Option<Value>,
            #[serde(default)]
            resources: Vec<ResourceRef>,
            #[serde(default)]
            options: Option<Value>,
        }

        let helper = AiPayloadHelper::deserialize(deserializer)?;
        let mut payload = Self {
            text: None,
            messages: vec![],
            tool_specs: vec![],
            resources: helper.resources,
            input_json: helper.input_json,
            options: helper.options,
        };
        if let Some(body) = payload
            .input_json
            .as_ref()
            .and_then(|value| value.as_object())
        {
            payload.text = body
                .get("text")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            payload.messages = body
                .get("messages")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_default();
            payload.tool_specs = body
                .get("tool_specs")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_default();
        }
        Ok(payload)
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiToolCall {
    pub name: String,
    pub args: HashMap<String, Value>,
    pub call_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiResponseSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<AiToolCall>,
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
pub struct AiMethodRequest {
    pub capability: Capability,
    pub model: ModelSpec,
    pub requirements: Requirements,
    pub payload: AiPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<RoutePolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiTaskOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
}

impl AiMethodRequest {
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
            policy: None,
            idempotency_key,
            task_options: None,
        }
    }

    pub fn with_policy(mut self, policy: Option<RoutePolicy>) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_task_options(mut self, task_options: Option<AiTaskOptions>) -> Self {
        self.task_options = task_options;
        self
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse AiMethodRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiMethodStatus {
    Succeeded,
    Running,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiMethodResponse {
    pub task_id: String,
    pub status: AiMethodStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AiResponseSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
}

impl AiMethodResponse {
    pub fn new(
        task_id: String,
        status: AiMethodStatus,
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

    pub async fn call_method(
        &self,
        method: &str,
        request: AiMethodRequest,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        if !ai_methods::is_ai_method(method) {
            return Err(RPCErrors::UnknownMethod(method.to_string()));
        }

        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_method(method, request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize AiMethodRequest: {}",
                        error
                    ))
                })?;
                let result = client.call(method, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse AI method response: {}",
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
    async fn handle_method(
        &self,
        method: &str,
        request: AiMethodRequest,
        ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors>;

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

        let method = req.method.clone();
        let result = match method.as_str() {
            ai_methods::CANCEL => {
                let cancel_req = CancelRequest::from_json(req.params)?;
                let result = self.0.handle_cancel(&cancel_req.task_id, ctx).await?;
                RPCResult::Success(json!(result))
            }
            method if ai_methods::is_ai_method(method) => {
                let method_req = AiMethodRequest::from_json(req.params)?;
                let result = self.0.handle_method(method, method_req, ctx).await?;
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
        method: Option<String>,
        request: Option<AiMethodRequest>,
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
        async fn handle_method(
            &self,
            method: &str,
            request: AiMethodRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<AiMethodResponse, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.method = Some(method.to_string());
            calls.request = Some(request);
            Ok(AiMethodResponse::new(
                "task-001".to_string(),
                AiMethodStatus::Succeeded,
                Some(AiResponseSummary {
                    text: Some("mock result".to_string()),
                    tool_calls: vec![],
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

    fn sample_method_request() -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.plan.default".to_string(), None),
            Requirements::new(vec![features::PLAN.to_string()], Some(3000), None, None),
            AiPayload::new(
                Some("write a release note".to_string()),
                vec![AiMessage::new(
                    "user".to_string(),
                    "summarize this commit".to_string(),
                )],
                vec![],
                vec![
                    ResourceRef::url(
                        "cyfs://example/object/1".to_string(),
                        Some("text/plain".to_string()),
                    ),
                    ResourceRef::named_object(ObjId::new("chunk:123456").unwrap()),
                ],
                Some(json!({
                    "messages": [
                        {
                            "role": "user",
                            "content": "summarize this commit"
                        }
                    ],
                    "text": "write a release note"
                })),
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

    #[test]
    fn test_protocol_field_names() {
        let mut request = sample_method_request();
        request.requirements.resp_format = RespFormat::Json;
        request.policy = Some(RoutePolicy::default());

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value.pointer("/capability"), Some(&json!("llm")));
        assert_eq!(
            value.pointer("/requirements/resp_format"),
            Some(&json!("json"))
        );
        assert!(value.pointer("/requirements/resp_foramt").is_none());
        assert!(value.pointer("/payload/text").is_none());
        assert!(value.pointer("/payload/messages").is_none());
        assert!(value.pointer("/payload/tool_specs").is_none());
        assert_eq!(
            value.pointer("/payload/input_json/messages/0/content"),
            Some(&json!("summarize this commit"))
        );
    }

    #[tokio::test]
    async fn test_in_process_client_with_mock() {
        let mock = MockAicc::new();
        let calls = mock.calls.clone();
        let client = AiccClient::new_in_process(Box::new(mock));

        let request = sample_method_request();
        let method_result = client
            .call_method(ai_methods::LLM_CHAT, request.clone())
            .await
            .unwrap();
        assert_eq!(method_result.task_id, "task-001");
        assert_eq!(method_result.status, AiMethodStatus::Succeeded);
        assert_eq!(
            method_result
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
        assert_eq!(calls.method.as_deref(), Some(ai_methods::LLM_CHAT));
        assert_eq!(calls.request, Some(request));
        assert_eq!(calls.cancel_task_id.as_deref(), Some("task-001"));
    }

    #[tokio::test]
    async fn test_rpc_handler_adapter_with_mock() {
        let mock = MockAicc::new();
        let calls = mock.calls.clone();
        let rpc_handler = AiccServerHandler::new(mock);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        let request = sample_method_request();
        let method_req = RPCRequest {
            method: ai_methods::LLM_CHAT.to_string(),
            params: serde_json::to_value(&request).unwrap(),
            seq: 9,
            token: None,
            trace_id: None,
        };
        let method_resp = rpc_handler.handle_rpc_call(method_req, ip).await.unwrap();
        match method_resp.result {
            RPCResult::Success(value) => {
                let method_result: AiMethodResponse = serde_json::from_value(value).unwrap();
                assert_eq!(method_result.task_id, "task-001");
                assert_eq!(method_result.status, AiMethodStatus::Succeeded);
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
        assert_eq!(calls.method.as_deref(), Some(ai_methods::LLM_CHAT));
        assert_eq!(calls.request, Some(request));
        assert_eq!(calls.cancel_task_id.as_deref(), Some("task-001"));
    }
}
