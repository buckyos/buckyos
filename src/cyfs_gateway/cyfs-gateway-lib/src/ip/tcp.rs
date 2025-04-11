use crate::tunnel::{StreamListener, TunnelEndpoint};
use crate::{TunnelError, TunnelResult};
use buckyos_kit::AsyncStream;
use url::Url;

pub struct TcpStreamListener {
    bind_addr: Url,
    listener: Option<tokio::net::TcpListener>,
}

impl TcpStreamListener {
    pub fn new(bind_addr: &Url) -> TcpStreamListener {
        TcpStreamListener {
            bind_addr: bind_addr.clone(),
            listener: None,
        }
    }

    pub async fn bind(&mut self) -> TunnelResult<()> {
        let host = self.bind_addr.host_str().unwrap();
        let port = self.bind_addr.port().unwrap();
        let bind_str = format!("{}:{}", host, port);
        info!("TcpStreamListener try bind to {}", bind_str);
        let listener = tokio::net::TcpListener::bind(bind_str.as_str())
            .await
            .map_err(|e| TunnelError::BindError(e.to_string()))?;
        info!("TcpStreamListener bind to {} OK", bind_str);
        self.listener = Some(listener);
        Ok(())
    }
}

#[async_trait::async_trait]
impl StreamListener for TcpStreamListener {
    async fn accept(&self) -> Result<(Box<dyn AsyncStream>, TunnelEndpoint), std::io::Error> {
        let listener = self.listener.as_ref().unwrap();
        let (stream, addr) = listener.accept().await?;
        Ok((
            Box::new(stream),
            TunnelEndpoint {
                device_id: addr.ip().to_string(),
                port: addr.port(),
            },
        ))
    }
}
