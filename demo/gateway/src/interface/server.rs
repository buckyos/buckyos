use gateway_lib::*;
use crate::proxy::{ForwardProxyConfig, ProxyConfig, ProxyManagerRef};
use crate::service::{UpstreamManagerRef, UpstreamService};
use crate::storage::ConfigStorageRef;

use std::net::SocketAddr;
use std::sync::Arc;
use warp::Filter;

#[derive(Clone)]
pub struct GatewayInterface {
    upstream_manager: UpstreamManagerRef,
    proxy_manager: ProxyManagerRef,

    addr: SocketAddr,

    config_storage: ConfigStorageRef,
}

impl GatewayInterface {
    pub fn new(upstream_manager: UpstreamManagerRef, proxy_manager: ProxyManagerRef, config_storage: ConfigStorageRef) -> Self {
        let addr = format!("127.0.0.1:{}", HTTP_INTERFACE_DEFAULT_PORT);
        let addr = addr.parse().unwrap();

        Self {
            upstream_manager,
            proxy_manager,

            addr,

            config_storage,
        }
    }

    async fn on_add_upstream(&self, body: serde_json::Value) -> GatewayResult<()> {
        let service: UpstreamService = UpstreamService::load(&body, Some(ConfigSource::Dynamic))?;

        self.upstream_manager.add(service)?;

        self.config_storage.notify_config_change();

        Ok(())
    }

    async fn on_remove_upstream(&self, body: serde_json::Value) -> GatewayResult<()> {
        let id = body.get("id").unwrap().as_str().ok_or_else(|| {
            GatewayError::InvalidConfig("Invalid request id not found".to_owned())
        })?;

        self.upstream_manager.remove(id)?;

        self.config_storage.notify_config_change();

        Ok(())
    }

    async fn on_add_sock5_proxy(&self, body: serde_json::Value) -> GatewayResult<()> {
        let config = ProxyConfig::load(&body, Some(ConfigSource::Dynamic))?;

        self.proxy_manager.create_socks5_proxy(config).await?;

        self.config_storage.notify_config_change();

        Ok(())
    }

    async fn on_add_forward_proxy(&self, body: serde_json::Value) -> GatewayResult<()> {
        let config = ForwardProxyConfig::load(&body, Some(ConfigSource::Dynamic))?;

        self.proxy_manager.create_tcp_forward_proxy(config).await?;

        self.config_storage.notify_config_change();

        Ok(())
    }

    async fn on_remove_proxy(&self, body: serde_json::Value) -> GatewayResult<()> {
        let id = body.get("id").unwrap().as_str().ok_or_else(|| {
            GatewayError::InvalidConfig("Invalid request id not found".to_owned())
        })?;

        self.proxy_manager.remove_proxy(id)?;

        self.config_storage.notify_config_change();

        Ok(())
    }

    fn ret_to_response(ret: GatewayResult<()>) -> warp::reply::Response {
        match ret {
            Ok(_) => {
                let mut resp = warp::reply::Response::new(
                    "{
                    \"ret\": \"success\"
                }"
                    .into(),
                );
                *resp.status_mut() = warp::http::StatusCode::OK;

                resp
            }
            Err(e) => {
                let status = gateway_lib::error_to_status_code(&e);

                let msg = format!("{}", e);
                let reply = format!("{{\"ret\": \"failed\", \"msg\": \"{}\"}}", msg);
                let mut resp = warp::reply::Response::new(reply.into());
                *resp.status_mut() = status;

                resp
            }
        }
    }

    pub async fn start(&self) -> GatewayResult<()> {
        let this = std::sync::Arc::new(self.clone());
        let service_filter = warp::any().map(move || this.clone());

        let add_upstream_route = warp::path!("service" / "upstream")
            .and(warp::post())
            .and(warp::body::json())
            .and(service_filter.clone())
            .then(
                |body: serde_json::Value, this: std::sync::Arc<Self>| async move {
                    let ret = this.on_add_upstream(body).await;
                    Self::ret_to_response(ret)
                },
            );

        let remove_upstream_route = warp::path!("service" / "upstream")
            .and(warp::delete())
            .and(warp::body::json())
            .and(service_filter.clone())
            .then(|body: serde_json::Value, this: Arc<Self>| async move {
                let ret = this.on_remove_upstream(body).await;
                Self::ret_to_response(ret)
            });

        let add_socks5_proxy_route = warp::path!("service" / "proxy" / "socks5")
            .and(warp::post())
            .and(warp::body::json())
            .and(service_filter.clone())
            .then(
                |body: serde_json::Value, this: std::sync::Arc<Self>| async move {
                    let ret = this.on_add_sock5_proxy(body).await;
                    Self::ret_to_response(ret)
                },
            );

        let add_forward_proxy_route = warp::path!("service" / "proxy" / "forward")
            .and(warp::post())
            .and(warp::body::json())
            .and(service_filter.clone())
            .then(
                |body: serde_json::Value, this: std::sync::Arc<Self>| async move {
                    let ret = this.on_add_forward_proxy(body).await;
                    Self::ret_to_response(ret)
                },
            );

        let remove_proxy_route = warp::path!("service" / "proxy")
            .and(warp::delete())
            .and(warp::body::json())
            .and(service_filter.clone())
            .then(
                |body: serde_json::Value, this: std::sync::Arc<Self>| async move {
                    let ret = this.on_remove_proxy(body).await;
                    Self::ret_to_response(ret)
                },
            );

        let routes = add_upstream_route
            .or(remove_upstream_route)
            .or(add_socks5_proxy_route)
            .or(add_forward_proxy_route)
            .or(remove_proxy_route);

        let server = warp::serve(routes);
        let (addr, server) = server.try_bind_ephemeral(self.addr.clone()).map_err(|e| {
            let msg = format!("Error binding http interface server: {}, {}", self.addr, e);
            error!("{}", msg);
            GatewayError::AlreadyExists(msg)
        })?;

        info!("Gateway interface server listening on {}", addr);

        tokio::spawn(server);

        Ok(())
    }
}
