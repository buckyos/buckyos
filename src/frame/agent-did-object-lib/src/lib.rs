pub mod adapters;
pub mod config;
pub mod error;
pub mod event_bridge;
pub mod router;
pub mod runtime;
pub mod tool;
pub mod types;

pub use adapters::{
    AdapterEventSubscription, AdapterEventTransport, AdapterReadRequest, AdapterReadResponse,
    AdapterRegistry, AdapterSubscribeEventRequest, AdapterUnsubscribeEventRequest,
    AdapterXCallRequest, AdapterXCallResponse, AgentObjectAdapter,
};
pub use agent_tool::{AgentToolResult, AgentToolStatus};
pub use config::{AdapterConfig, AdapterType, ObjectRoute, ObjectRouteConfig};
pub use error::AgentDIDObjectError;
pub use event_bridge::{
    bridge_key, encode_object_event_id, event_frame_payload, filter_hash, BridgeState,
    EventBridgeKey, EventBridgeManager, EventBridgeSink, EventBridgeSnapshot, EventBridgeStart,
    EventTransport, EventTransportHandle, EventTransportStarted, NoopEventTransport,
    WebSocketEventTransport,
};
pub use router::{ObjectRouter, RouteMatch, RouteMatchType, RouteMethod, RouteTrace};
pub use runtime::AgentDIDObjectRuntime;
pub use tool::{AgentDIDObjectReadTool, TOOL_READ};
pub use types::*;
