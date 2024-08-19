use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};


#[async_trait::async_trait]
pub trait Tunnel: Send + Unpin + AsyncRead + AsyncWrite {
    fn split(self: Box<Self>) -> (Box<dyn TunnelReader>, Box<dyn TunnelWriter>);

    /*
    async fn run_forward(&mut self, forward: String) -> GatewayResult<()> {
        let mut stream = TcpStream::connect(&forward).await.map_err(|e| {
            error!("Error connecting to forward address {}: {}", forward, e);
            e
        })?;

        let (mut tunnel_reader, mut tunnel_writer) = self.split();
        let (mut stream_reader, mut stream_writer) = stream.split();

        let tunnel_to_stream = tokio::io::copy(&mut tunnel_reader, &mut stream_writer);
        let stream_to_tunnel = tokio::io::copy(&mut stream_reader, &mut tunnel_writer);

        tokio::try_join!(tunnel_to_stream, stream_to_tunnel).unwrap();

        Ok(())
    }
    */
}

pub trait TunnelReader: Send + Unpin + AsyncRead {}

pub trait TunnelWriter: Send + Unpin + AsyncWrite {}

impl<T: AsyncRead + Unpin + Send> TunnelReader for T {}
impl<T: AsyncWrite + Unpin + Send> TunnelWriter for T {}

// impl<T: AsyncRead + AsyncWrite + Unpin + Send> Tunnel for T {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelType {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelSide {
    Active,
    Passive,
}

pub struct TunnelCombiner {
    pub reader: Box<dyn TunnelReader>,
    pub writer: Box<dyn TunnelWriter>,
}

impl TunnelCombiner {
    pub fn new(reader: Box<dyn TunnelReader>, writer: Box<dyn TunnelWriter>) -> Self {
        Self { reader, writer }
    }
}

impl Tunnel for TunnelCombiner {
    fn split(self: Box<Self>) -> (Box<dyn TunnelReader>, Box<dyn TunnelWriter>) {
        (self.reader, self.writer)
    }
}

impl AsyncRead for TunnelCombiner {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.get_mut().reader).poll_read(cx, buf)
    }
}

impl AsyncWrite for TunnelCombiner {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut *self.get_mut().writer).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.get_mut().writer).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.get_mut().writer).poll_shutdown(cx)
    }
}