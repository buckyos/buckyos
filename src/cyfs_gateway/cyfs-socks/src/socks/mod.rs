mod config;
mod socks5;
mod util;


pub use config::*;
pub use socks5::*;

use crate::error::SocksResult;

// This function is used to start the socks server and will not block the current thread.
// If start successfully, it will return Ok(()), otherwise return an error.
pub async fn start_cyfs_socks_server(config: SocksProxyConfig) -> SocksResult<()> {
    let server = Socks5Proxy::new(config);
    server.start().await
}

pub async fn stop_cyfs_socks_server(_id: &str) -> SocksResult<()> {
    // TODO We should manager the socks server by id, then we can stop it.
    todo!("stop_socks_server");
}