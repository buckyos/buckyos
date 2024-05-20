use crate::error::GatewayError;

use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ProxyAuth {
    None,
    Password(String, String),
}

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub addr: SocketAddr,
    pub auth: ProxyAuth,
}

#[async_trait::async_trait]
pub trait GatewayProxy {
    async fn start(&self) -> Result<(), GatewayError>;
    async fn stop(&self) -> Result<(), GatewayError>;
}

#[async_trait::async_trait]
pub trait GatewayProxyEvents: Send + Sync {
    async fn on_new_connection<S>(&self, socket: S) -> Result<(), GatewayError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static;
}

pub type GatewayProxyEventsRef = Arc<Box<dyn GatewayProxyEvents>>;