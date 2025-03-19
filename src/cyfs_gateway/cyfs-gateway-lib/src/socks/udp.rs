use crate::tunnel::DatagramClient;
use libsocks_client::SocksUdpSocket;
use std::sync::Arc;

#[derive(Clone)]
pub struct SocksUdpClient {
    socket: Arc<SocksUdpSocket>,
    dest_port: u16,
    dest_addr: String,
}

impl SocksUdpClient {
    pub fn new(socket: SocksUdpSocket, dest_addr: String, dest_port: u16) -> Self {
        Self {
            socket: Arc::new(socket),
            dest_port,
            dest_addr,
        }
    }
}

#[async_trait::async_trait]
impl DatagramClient for SocksUdpClient {
    async fn recv_datagram(&self, buffer: &mut [u8]) -> Result<usize, std::io::Error> {
        let (addr, data) = self.socket.recv_udp_data(60).await.map_err(|e| {
            let msg = format!("Failed to recv_udp_data: {}", e);
            error!("{}", msg);
            std::io::Error::new(std::io::ErrorKind::Other, msg)
        })?;

        if data.len() > buffer.len() {
            let msg = format!(
                "buffer is too small, data len: {}, buffer len: {}",
                data.len(),
                buffer.len()
            );
            error!("{}", msg);
            return Err(std::io::Error::new(std::io::ErrorKind::OutOfMemory, msg));
        }

        buffer[..data.len()].copy_from_slice(&data);

        debug!(
            "Socks udp client recv datagram from {} size: {}",
            addr,
            data.len()
        );

        Ok(data.len())
    }

    async fn send_datagram(&self, buffer: &[u8]) -> Result<usize, std::io::Error> {
        let dest_addr = format!("{}:{}", self.dest_addr, self.dest_port);
        let size = self
            .socket
            .send_udp_data(buffer, &dest_addr)
            .await
            .map_err(|e| {
                let msg = format!("Failed to send_udp_data: {}", e);
                error!("{}", msg);
                std::io::Error::new(std::io::ErrorKind::Other, msg)
            })?;

        debug!(
            "Socks udp client send datagram to {} size: {}",
            dest_addr, size
        );
        Ok(size)
    }
}
