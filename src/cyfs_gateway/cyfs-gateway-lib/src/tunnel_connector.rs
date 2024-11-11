use hyper::client::connect::{Connected, Connection};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use url::Url;
use std::error::Error as StdError;
use std::task::{Context, Poll};
use hyper::service::Service;
use crate::AsyncStream;
use std::pin::Pin;
use std::future::Future;
use hyper::Uri;
use log::*;
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

impl Service<Uri> for TunnelConnector {
    type Response = TunnelStreamConnection;
    type Error = Box<dyn StdError + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(async move {
            //info!("HTTP upstream TunnelConnector will open stream to {}", uri.to_string());
            // let target = Url::parse(uri.to_string().as_str())
            //     .map_err(|e| Box::new(e) as Box<dyn StdError + Send + Sync>)?;

            // let tunnel_host = target.host();
            // let target_port = target.port();
            // if tunnel_host.is_none() {
            //     warn!("TunnelConnector Get tunnel failed! {}", "tunnel host is none");
            //     return Err(anyhow::anyhow!("TunnelConnector Get tunnel failed!").context("tunnel host is none").into()); 
            // }

            // let tunnel_host = tunnel_host.unwrap();
            // let tunnel_url = format!("rtcp://{}",tunnel_host);
            // let tunnel_url = Url::parse(&tunnel_url).unwrap();
            let target_url = Url::parse(uri.to_string().as_str()).unwrap();
            let target_tunnel = get_tunnel(&target_url, None).await;
            if let Err(err) = target_tunnel {
                warn!("TunnelConnector Get tunnel failed! {}", err);
                return Err(Box::new(err) as Box<dyn StdError + Send + Sync>);
            }
            let target_port = uri.port_u16().unwrap_or(80);
            let target_tunnel = target_tunnel.unwrap();
            //info!("TunnelConnector Get tunnel OK! {}", target.to_string());
            let target_stream = target_tunnel.open_stream(target_port).await
                .map_err(|e| Box::new(e) as Box<dyn StdError + Send + Sync>)?;
            Ok(TunnelStreamConnection::new(target_stream))
        })
    }
}
