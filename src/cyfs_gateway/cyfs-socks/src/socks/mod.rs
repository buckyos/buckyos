mod config;
mod socks5;
mod util;

pub use config::*;
pub use fast_socks5::util::target_addr::TargetAddr;
pub use socks5::*;

use crate::error::SocksResult;
use once_cell::sync::OnceCell;

// The main socks server, only one per process
// TODO If there any case we need to support multiple socks server? at that case we can use a HashMap to store the socks server.
static MAIN_SOCKS_SERVER: OnceCell<Socks5Proxy> = OnceCell::new();

// This function is used to start the socks server and will not block the current thread.
// If start successfully, it will return Ok(()), otherwise return an error.
pub async fn start_cyfs_socks_server(
    config: SocksProxyConfig,
    tunnel_provider: crate::SocksDataTunnelProviderRef,
) -> SocksResult<()> {
    let server = Socks5Proxy::new(config);
    server.start().await?;

    server.set_data_tunnel_provider(tunnel_provider);

    if let Err(_) = MAIN_SOCKS_SERVER.set(server) {
        unreachable!("MAIN_SOCKS_SERVER should be set only once!");
    }

    Ok(())
}

pub async fn stop_cyfs_socks_server(_id: &str) -> SocksResult<()> {
    // TODO We should manager the socks server by id, then we can stop it.
    todo!("stop_socks_server");
}

pub fn get_main_socks_server() -> &'static Socks5Proxy {
    MAIN_SOCKS_SERVER
        .get()
        .expect("MAIN_SOCKS_SERVER should be start before get")
}
