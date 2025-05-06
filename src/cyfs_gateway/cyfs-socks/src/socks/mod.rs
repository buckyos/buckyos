mod config;
mod socks5;
mod util;

pub use config::*;
pub use fast_socks5::util::target_addr::TargetAddr;
pub use socks5::*;

use crate::error::SocksResult;


// This function is used to start the socks server and will not block the current thread.
// If start successfully, it will return Ok(()), otherwise return an error.
pub async fn start_cyfs_socks_server(
    config: SocksProxyConfig,
    tunnel_provider: crate::SocksDataTunnelProviderRef,
) -> SocksResult<Socks5Proxy> {
    let server = Socks5Proxy::new(config);
    server.start().await?;

    server.set_data_tunnel_provider(tunnel_provider);

    Ok(server)
}
