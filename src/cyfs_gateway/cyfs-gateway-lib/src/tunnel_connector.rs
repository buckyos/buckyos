use hyper::client::connect::{Connected, Connection};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use url::Url;
use std::error::Error as StdError;
use std::task::{Context, Poll};
use hyper::service::Service;
use buckyos_kit::AsyncStream;
use std::pin::Pin;
use std::future::Future;
use hyper::Uri;
use log::*;
use crate::tunnel_mgr::*;


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
        //info!("TunnelStreamConnection poll_read len:{}",buf.filled().len());
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
        //info!("TunnelStreamConnection poll_write len:{}",buf.len());
        Pin::new(&mut *self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        //info!("TunnelStreamConnection poll_flush");
        Pin::new(&mut *self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        //info!("TunnelStreamConnection poll_shutdown");
        Pin::new(&mut *self.get_mut().inner).poll_shutdown(cx)
    }
}


#[derive(Clone)]
pub struct TunnelConnector;

//open stream by url
impl Service<Uri> for TunnelConnector {
    type Response = TunnelStreamConnection;
    type Error = Box<dyn StdError + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(async move {
            let stream_url = Url::parse(&uri.to_string()).unwrap();
            let target_stream = open_stream_by_url(&stream_url).await.map_err(|e| {
                warn!("TunnelConnector open_stream_by_url  failed! {}", e);
                Box::new(e) as Box<dyn StdError + Send + Sync>
            })?;
                
            Ok(TunnelStreamConnection::new(target_stream))
        })
    }
}
