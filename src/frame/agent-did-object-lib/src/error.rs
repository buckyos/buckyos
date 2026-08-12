use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentDIDObjectError {
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("route not found: {0}")]
    RouteNotFound(String),
    #[error("adapter not found: {0}")]
    AdapterNotFound(String),
    #[error("adapter unavailable: {0}")]
    AdapterUnavailable(String),
    #[error("unsupported object ref: {0}")]
    UnsupportedObjectRef(String),
    #[error("unsupported method: {0}")]
    UnsupportedMethod(String),
    #[error("resolve error: {0}")]
    ResolveError(String),
    #[error("declared capability not found: {0}")]
    DeclaredCapabilityNotFound(String),
    #[error("schema error: {0}")]
    SchemaError(String),
    #[error("http error: {0}")]
    HttpError(String),
    #[error("protocol error: {0}")]
    ProtocolError(String),
    #[error("kevent error: {0}")]
    KEventError(String),
    #[error("event bridge error: {0}")]
    EventBridgeError(String),
    #[error("adapter error: {0}")]
    AdapterError(String),
}

impl From<reqwest::Error> for AgentDIDObjectError {
    fn from(err: reqwest::Error) -> Self {
        Self::HttpError(err.to_string())
    }
}

impl From<std::io::Error> for AgentDIDObjectError {
    fn from(err: std::io::Error) -> Self {
        Self::AdapterError(err.to_string())
    }
}

impl From<toml::de::Error> for AgentDIDObjectError {
    fn from(err: toml::de::Error) -> Self {
        Self::InvalidConfig(err.to_string())
    }
}
