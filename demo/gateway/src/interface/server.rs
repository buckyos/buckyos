use crate::error::{GatewayError, GatewayResult};
use crate::proxy::{ProxyManagerRef, ProxyConfig, ForwardProxyConfig};
use crate::service::{UpstreamManagerRef, UpstreamService};


use tokio::net::{TcpListener, TcpStream};
use std::net::SocketAddr;
use tokio_util::codec::{Framed};


pub struct GatewayInterface {
    upstream_manager: UpstreamManagerRef,
    proxy_manager: ProxyManagerRef,

    addr: SocketAddr,
}

impl GatewayInterface {
    pub fn new(upstream_manager: UpstreamManagerRef, proxy_manager: ProxyManagerRef) -> Self {
        let addr = "127.0.0.1:23008".parse().unwrap();

        Self {
            upstream_manager,
            proxy_manager,

            addr,
        }
    }

    async fn process_req(&self, req: Request<Incoming>) -> GatewayResult<()> {
        let body = hyper::body::to_bytes(req.into_body()).await.map_err(|e| {
            let msg = format!("Error reading request body: {}", e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        let body = String::from_utf8(body.to_vec()).map_err(|e| {
            let msg = format!("Error parsing request body: {}", e);
            error!("{}", msg);
            GatewayError::InvalidFormat(msg)
        })?;

        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            let msg = format!("Error parsing request json body: {}", e);
            error!("{}", msg);
            GatewayError::InvalidFormat(msg)
        })?;

        match req.uri().path() {
            "/service/upstream" => {
                match req.method() {
                    &Method::POST => {
                        let service = UpstreamService::load(&json)?;

                        self.upstream_manager.add(service)?;
                    }
                    &Method::DELETE => {
                        let id = json.get("id").unwrap().as_str().ok_or_else(|| {
                            GatewayError::InvalidConfig("Invalid request id not found".to_owned())
                        })?;
                        self.upstream_manager.remove(id)?;
                    }
                    _ => {
                        return Err(GatewayError::InvalidParam("Invalid request".to_owned()));
                    }
                }
            }
            "/service/proxy/socks5" => {
                match req.method() {
                    &Method::POST => {
                        let config = ProxyConfig::load(&json)?;

                        self.proxy_manager.create_socks5_proxy(config).await?;
                    }
                    &Method::DELETE => {
                        let id = json.get("id").unwrap().as_str().ok_or_else(|| {
                            GatewayError::InvalidConfig("Invalid request id not found".to_owned())
                        })?;
                        self.proxy_manager.remove_proxy(id)?;
                    }
                    _ => {
                        return Err(GatewayError::InvalidParam("Invalid request".to_owned()));
                    }
                }
            }
            "/service/proxy/forward" => {
                match req.method() {
                    &Method::POST => {
                        let config = ForwardProxyConfig::load(&json)?;

                        self.proxy_manager.create_tcp_forward_proxy(config).await?;
                    }
                    &Method::DELETE => {
                        let id = json.get("id").unwrap().as_str().ok_or_else(|| {
                            GatewayError::InvalidConfig("Invalid request id not found".to_owned())
                        })?;
                        self.proxy_manager.remove_proxy(id)?;
                    }
                    _ => {
                        return Err(GatewayError::InvalidParam("Invalid request".to_owned()));
                    }
                }
            }
            _ => {
                let msg = format!("Invalid request path: {}", req.uri().path());
                warn!("{}", msg);
                return Err(GatewayError::NotFound(msg));
            }
        }

        Ok(())
    }

    pub async fn start(&self) -> GatewayResult<()> {

        let server = TcpListener::bind(&self.addr).await.map_err(|e| {
            let msg = format!("Error binding http interface server: {}, {}", self.addr, e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        loop {
            let (stream, addr) = server.accept().await.map_err(|e| {
                let msg = format!("Error accepting http interface server: {}, {}", self.addr, e);
                error!("{}", msg);
                GatewayError::Io(e)
            })?;

    
            let framed = Framed::new(stream, Default::default());
        }
        
        Ok(())
    }

    async fn process_connection(&self, stream: TcpStream) -> GatewayResult<()> {
        let mut transport = Framed::new(stream, Http);

    while let Some(request) = transport.next().await {
        match request {
            Ok(request) => {
                let response = respond(request).await?;
                transport.send(response).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())

        Ok(())
    }
}