use buckyos_kit::AsyncStream;
use cyfs_gateway_lib::get_tunnel;
use cyfs_socks::{
    SocksDataTunnelProvider, SocksDataTunnelProviderRef, SocksError, SocksResult, TargetAddr,
};
use std::sync::Arc;
use url::Url;

pub struct SocksTunnelBuilder {}

impl SocksTunnelBuilder {
    pub fn new_ref() -> SocksDataTunnelProviderRef {
        Arc::new(Box::new(Self {}))
    }
}

#[async_trait::async_trait]
impl SocksDataTunnelProvider for SocksTunnelBuilder {
    async fn build(
        &self,
        target: &TargetAddr,
        request_target: &Url,
        enable_tunnel: &Option<Vec<String>>,
    ) -> SocksResult<Box<dyn AsyncStream>> {
        info!(
            "Will build tunnel for request: {:?}, {:?}",
            target, request_target
        );
        let target_tunnel = get_tunnel(request_target, enable_tunnel.clone())
            .await
            .map_err(|e| {
                let msg = format!(
                    "Get tunnel to proxy target failed: {}, {:?}",
                    request_target, e
                );
                error!("{}", msg);
                SocksError::IoError(msg)
            })?;

        let target_port = request_target.port().unwrap_or(0);
        if target_port == 0 {
            let msg = format!("Invalid target port: {:?}", request_target);
            error!("{}", msg);
            return Err(SocksError::InvalidConfig(msg));
        }

        let target_host = match &target {
            TargetAddr::Ip(ip) => ip.ip().to_string(),
            TargetAddr::Domain(domain, _) => domain.clone(),
        };

        let target_stream = target_tunnel.open_stream(target_port, Some(target_host)).await.map_err(|e| {
            let msg = format!("Open target stream failed: {}, {:?}", target, e);
            error!("{}", msg);
            SocksError::IoError(msg)
        })?;

        Ok(target_stream)
    }
}
