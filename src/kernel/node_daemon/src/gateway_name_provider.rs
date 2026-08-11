use std::time::Duration;

use cyfs_gateway_api::{GatewayControlClient, CONTROL_SERVER};
use log::*;

use crate::gateway_tunnel_probe::build_local_gateway_token;

// node-daemon 起的 cyfs-gateway 总是把本机 3180 端口 forward 到 zone 的
// system_config（详见 boot.rs 的 `_dispatch_to_system_config`）。把这个 URL
// 注册成 cyfs-gateway 的 name provider，等价于让 gateway 把 system_config
// 的 HttpsProvider 加到 GLOBAL_NAME_CLIENT，从而能 resolve 出 device 自报的
// IP / DeviceInfo。
const NODE_GATEWAY_NAME_PROVIDER_URL: &str = "http://127.0.0.1:3180/";
const REGISTER_RPC_TIMEOUT_SECS: u64 = 3;

/// 把本机 cyfs-gateway 的 system_config 转发口注册为 gateway 的 name
/// provider。返回 true 代表注册成功；任何失败（gateway 未就绪、token 拿不到、
/// RPC 错误）都返回 false，调用方负责重试。
pub async fn register_node_gateway_name_provider() -> bool {
    let token = match build_local_gateway_token() {
        Some(t) => t,
        None => {
            debug!("skip register node_gateway name provider: no gateway control token");
            return false;
        }
    };

    let client = GatewayControlClient::new(CONTROL_SERVER, Some(token));
    match tokio::time::timeout(
        Duration::from_secs(REGISTER_RPC_TIMEOUT_SECS),
        client.add_name_provider(NODE_GATEWAY_NAME_PROVIDER_URL, None),
    )
    .await
    {
        Ok(Ok(_)) => {
            info!(
                "registered node_gateway as cyfs-gateway name provider: {}",
                NODE_GATEWAY_NAME_PROVIDER_URL
            );
            true
        }
        Ok(Err(err)) => {
            debug!(
                "register node_gateway name provider failed (gateway may not be ready): {:?}",
                err
            );
            false
        }
        Err(_) => {
            debug!(
                "register node_gateway name provider rpc timed out after {}s",
                REGISTER_RPC_TIMEOUT_SECS
            );
            false
        }
    }
}
