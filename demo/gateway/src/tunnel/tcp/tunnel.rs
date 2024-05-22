use std::net::SocketAddr;

use super::super::tunnel::{Tunnel, TunnelReader, TunnelWriter};
use crate::error::*;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

pub struct TcpTunnel {
    remote: SocketAddr,
    stream: TcpStream,
}

impl TcpTunnel {
    pub fn new(remote: SocketAddr, stream: TcpStream) -> Self {
        Self { remote, stream }
    }
    
    pub fn remote(&self) -> &SocketAddr {
        &self.remote
    }

    pub async fn build(remote: SocketAddr) -> GatewayResult<Self> {
        let stream = TcpStream::connect(&remote).await.map_err(|e| {
            error!("Error connecting to remote {}: {}", remote, e);
            e
        })?;

        info!("Tcp tunnel connected to remote {}", remote);
        Ok(Self { remote, stream })
    }

    pub fn split(self) -> (Box<dyn TunnelReader>, Box<dyn TunnelWriter>) {
        let (reader, writer) = self.stream.into_split();
        (Box::new(reader), Box::new(writer))
    }
}

#[async_trait::async_trait]
impl Tunnel for TcpTunnel {
    fn split(self: Box<Self>) -> (Box<dyn TunnelReader>, Box<dyn TunnelWriter>) {
        TcpTunnel::split(*self)
    }
}

#[async_trait::async_trait]
impl AsyncRead for TcpTunnel {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

#[async_trait::async_trait]
impl AsyncWrite for TcpTunnel {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

