use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{AdapterConfig, AdapterType, ObjectRoute};
use crate::error::AgentDIDObjectError;
use crate::router::RouteTrace;
use crate::types::{
    EventBridgeSubscription, ObjectRef, PromptGuidance, ReadAttachedError, ReadInput, ReadMeta,
    SubscribeEventInput, TrustGuidance, UnsubscribeEventInput, XCallInput,
};

pub mod agent_runtime;
pub mod did_object;
pub mod filesystem;
pub mod local_http;
pub mod web;

pub use agent_runtime::AgentRuntimeAdapter;
pub use did_object::DidObjectProtocolAdapter;
pub use filesystem::FilesystemAdapter;
pub use local_http::LocalHttpAdapter;
pub use web::WebAdapter;

#[async_trait]
pub trait AgentObjectAdapter: Send + Sync {
    fn id(&self) -> &str;

    async fn read(
        &self,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError>;

    async fn x_call(
        &self,
        req: AdapterXCallRequest,
    ) -> Result<AdapterXCallResponse, AgentDIDObjectError>;

    async fn subscribe_event(
        &self,
        req: AdapterSubscribeEventRequest,
    ) -> Result<AdapterEventSubscription, AgentDIDObjectError>;

    async fn unsubscribe_event(
        &self,
        req: AdapterUnsubscribeEventRequest,
    ) -> Result<(), AgentDIDObjectError>;
}

#[derive(Clone, Debug)]
pub struct AdapterReadRequest {
    pub object_ref: ObjectRef,
    pub input: ReadInput,
    pub route: ObjectRoute,
    pub route_trace: RouteTrace,
    pub adapter_config: AdapterConfig,
}

#[derive(Clone, Debug)]
pub struct AdapterXCallRequest {
    pub object_ref: ObjectRef,
    pub input: XCallInput,
    pub route: ObjectRoute,
    pub route_trace: RouteTrace,
    pub adapter_config: AdapterConfig,
}

#[derive(Clone, Debug)]
pub struct AdapterSubscribeEventRequest {
    pub object_ref: ObjectRef,
    pub input: SubscribeEventInput,
    pub route: ObjectRoute,
    pub route_trace: RouteTrace,
    pub adapter_config: AdapterConfig,
}

#[derive(Clone, Debug)]
pub struct AdapterUnsubscribeEventRequest {
    pub input: UnsubscribeEventInput,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterReadResponse {
    pub object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub meta: ReadMeta,
    #[serde(default)]
    pub prompt_guidance: Vec<PromptGuidance>,
    #[serde(default)]
    pub trust_guidance: Vec<TrustGuidance>,
    #[serde(default)]
    pub errors: Vec<ReadAttachedError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub route: RouteTrace,
    #[serde(default)]
    pub adapt_meta: Value,
}

impl AdapterReadResponse {
    pub fn text(object: String, content: String, route: RouteTrace) -> Self {
        Self {
            object: object.clone(),
            object_did: None,
            content: Some(content),
            meta: ReadMeta::default(),
            prompt_guidance: vec![],
            trust_guidance: vec![],
            errors: vec![],
            cache_key: Some(object),
            version: None,
            route,
            adapt_meta: json!({}),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterXCallResponse {
    #[serde(default)]
    pub status: AdapterCallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default)]
    pub detail: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub route: RouteTrace,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterCallStatus {
    #[default]
    Success,
    Error,
    Pending,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterEventSubscription {
    pub subscription: EventBridgeSubscription,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<AdapterEventTransport>,
    #[serde(default = "default_adapter_unsubscribe")]
    pub unsubscribe_via_adapter: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdapterEventTransport {
    WebSocket {
        endpoint: String,
        subscribe: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unsubscribe: Option<Value>,
    },
}

fn default_adapter_unsubscribe() -> bool {
    true
}

#[derive(Clone, Default)]
pub struct AdapterRegistry {
    adapters: HashMap<String, Arc<dyn AgentObjectAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    pub fn with_builtin(config: &[AdapterConfig]) -> Result<Self, AgentDIDObjectError> {
        let mut registry = Self::new();
        for adapter in config {
            let instance: Arc<dyn AgentObjectAdapter> = match adapter.adapter_type {
                AdapterType::Filesystem => Arc::new(FilesystemAdapter::new(adapter.id.clone())),
                AdapterType::Web => Arc::new(WebAdapter::new(adapter.id.clone())),
                AdapterType::AgentRuntime => {
                    Arc::new(AgentRuntimeAdapter::new(adapter.id.clone(), None))
                }
                AdapterType::DidObject => {
                    Arc::new(DidObjectProtocolAdapter::new(adapter.id.clone()))
                }
                AdapterType::LocalHttp => {
                    Arc::new(LocalHttpAdapter::new(adapter.id.clone(), adapter.clone())?)
                }
            };
            registry.register(instance);
        }
        Ok(registry)
    }

    pub fn register(&mut self, adapter: Arc<dyn AgentObjectAdapter>) {
        self.adapters.insert(adapter.id().to_string(), adapter);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn AgentObjectAdapter>> {
        self.adapters.get(id).cloned()
    }
}

pub fn unsupported_event_subscription(
    adapter: &str,
    req: &AdapterSubscribeEventRequest,
) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
    Err(AgentDIDObjectError::UnsupportedMethod(format!(
        "adapter {adapter} does not support subscribe_event {}",
        req.input.event
    )))
}

pub fn unsupported_event_unsubscription(adapter: &str) -> Result<(), AgentDIDObjectError> {
    Err(AgentDIDObjectError::UnsupportedMethod(format!(
        "adapter {adapter} does not support unsubscribe_event"
    )))
}
