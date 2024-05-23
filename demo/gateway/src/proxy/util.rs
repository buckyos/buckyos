use fast_socks5::consts;
use fast_socks5::server::{SimpleUserPassword, Socks5Socket};
use fast_socks5::ReplyError;
use std::net::SocketAddr;
use tokio::io::AsyncWriteExt;

use crate::GatewayResult;

pub struct Socks5Util {}

impl Socks5Util {
    pub fn new_reply(error: ReplyError, sock_addr: SocketAddr) -> Vec<u8> {
        let (addr_type, mut ip_oct, mut port) = match sock_addr {
            SocketAddr::V4(sock) => (
                consts::SOCKS5_ADDR_TYPE_IPV4,
                sock.ip().octets().to_vec(),
                sock.port().to_be_bytes().to_vec(),
            ),
            SocketAddr::V6(sock) => (
                consts::SOCKS5_ADDR_TYPE_IPV6,
                sock.ip().octets().to_vec(),
                sock.port().to_be_bytes().to_vec(),
            ),
        };

        let mut reply = vec![
            consts::SOCKS5_VERSION,
            error.as_u8(), // transform the error into byte code
            0x00,          // reserved
            addr_type,     // address type (ipv4, v6, domain)
        ];
        reply.append(&mut ip_oct);
        reply.append(&mut port);

        reply
    }

    pub async fn reply_error(
        socket: &mut Socks5Socket<tokio::net::TcpStream, SimpleUserPassword>,
        error: ReplyError,
    ) -> GatewayResult<()> {
        let reply = Self::new_reply(error, "0.0.0.0:0".parse().unwrap());

        socket.write(&reply).await.map_err(|e| {
            error!("Error replying socks5 error: {}", e);
            e
        })?;

        socket.flush().await.map_err(|e| {
            error!("Error flushing socks5 error: {}", e);
            e
        })?;

        Ok(())
    }
}
