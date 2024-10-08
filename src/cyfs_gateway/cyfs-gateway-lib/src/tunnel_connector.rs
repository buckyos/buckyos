use hyper::client::connect::{Connected, Connection};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use url::Url;
use std::task::{Context, Poll};
use hyper::service::Service;
use crate::AsyncStream;
use std::pin::Pin;
use std::future::Future;
use hyper::Uri;
use crate::get_tunnel;


pub struct TunnelStreamConnection {
    inner: Box<dyn AsyncStream>
}

impl TunnelStreamConnection {
    pub fn new(inner: Box<dyn AsyncStream>) -> Self {
        TunnelStreamConnection { inner }
    }
}

impl Connection for TunnelStreamConnection {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

impl AsyncRead for TunnelStreamConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // 使用 Pin::new_unchecked 将 inner 的可变引用转换为 Pin<&mut dyn AsyncRead>
        Pin::new(&mut *self.get_mut().inner).poll_read(cx, buf)
    }
}


impl AsyncWrite for TunnelStreamConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut *self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.get_mut().inner).poll_shutdown(cx)
    }
}


#[derive(Clone)]
pub struct TunnelConnector;

impl Service<Uri> for TunnelConnector {
    type Response = TunnelStreamConnection;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(async move {
            let target = Url::parse(uri.to_string().as_str())?;
            let target_tunnel = get_tunnel(&target,None).await?;
            let target_stream = target_tunnel.open_stream(uri.port_u16().unwrap_or(80)).await?;
            Ok(TunnelStreamConnection::new(target_stream))
        })
    }
}
