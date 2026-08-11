use async_trait::async_trait;

use super::{
    unsupported_event_subscription, unsupported_event_unsubscription, AdapterEventSubscription,
    AdapterReadRequest, AdapterReadResponse, AdapterSubscribeEventRequest,
    AdapterUnsubscribeEventRequest, AdapterXCallRequest, AdapterXCallResponse, AgentObjectAdapter,
};
use crate::error::AgentDIDObjectError;

pub trait AgentRuntimeObjectHandle: Send + Sync {}

pub struct AgentRuntimeAdapter {
    id: String,
    handle: Option<std::sync::Arc<dyn AgentRuntimeObjectHandle>>,
}

impl AgentRuntimeAdapter {
    pub fn new(id: String, handle: Option<std::sync::Arc<dyn AgentRuntimeObjectHandle>>) -> Self {
        Self { id, handle }
    }
}

#[async_trait]
impl AgentObjectAdapter for AgentRuntimeAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    async fn read(
        &self,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        if self.handle.is_none() {
            return Err(AgentDIDObjectError::AdapterUnavailable(
                "agent_runtime adapter requires an explicit runtime handle".to_string(),
            ));
        }
        Err(AgentDIDObjectError::UnsupportedObjectRef(format!(
            "agent runtime object {} is not registered",
            req.object_ref.normalized
        )))
    }

    async fn x_call(
        &self,
        req: AdapterXCallRequest,
    ) -> Result<AdapterXCallResponse, AgentDIDObjectError> {
        if self.handle.is_none() {
            return Err(AgentDIDObjectError::AdapterUnavailable(
                "agent_runtime adapter requires an explicit runtime handle".to_string(),
            ));
        }
        Err(AgentDIDObjectError::UnsupportedMethod(format!(
            "agent runtime action {} is not registered",
            req.input.action
        )))
    }

    async fn subscribe_event(
        &self,
        req: AdapterSubscribeEventRequest,
    ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
        unsupported_event_subscription(&self.id, &req)
    }

    async fn unsubscribe_event(
        &self,
        _req: AdapterUnsubscribeEventRequest,
    ) -> Result<(), AgentDIDObjectError> {
        unsupported_event_unsubscription(&self.id)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{AdapterConfig, AdapterType, ObjectRoute};
    use crate::router::{RouteMatchType, RouteMethod, RouteTrace};
    use crate::types::{ObjectRef, ReadInput};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn no_runtime_handle_returns_unavailable() {
        let adapter = AgentRuntimeAdapter::new("agent-runtime".to_string(), None);
        let err = adapter
            .read(AdapterReadRequest {
                object_ref: ObjectRef::parse("agent://session/1").unwrap(),
                input: ReadInput {
                    object: "agent://session/1".to_string(),
                    purpose: None,
                    session_id: None,
                    content_only: false,
                    range: None,
                    max_tokens: None,
                    options: json!({}),
                },
                route: ObjectRoute {
                    id: "r".to_string(),
                    priority: 0,
                    match_type: RouteMatchType::Scheme,
                    pattern: "agent".to_string(),
                    adapter: "agent-runtime".to_string(),
                    methods: vec![RouteMethod::Read],
                    options: json!({}),
                },
                route_trace: RouteTrace {
                    route_id: "r".to_string(),
                    adapter: "agent-runtime".to_string(),
                    method: RouteMethod::Read,
                },
                adapter_config: AdapterConfig {
                    id: "agent-runtime".to_string(),
                    adapter_type: AdapterType::AgentRuntime,
                    endpoint: None,
                    auth_token_env: None,
                    options: json!({}),
                },
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AgentDIDObjectError::AdapterUnavailable(_)));
    }
}
