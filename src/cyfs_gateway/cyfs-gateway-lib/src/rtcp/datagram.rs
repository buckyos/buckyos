use crate::tunnel::DatagramClient;
use buckyos_kit::AsyncStream;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AsyncStreamWithDatagram {
    stream: Arc<Mutex<Box<dyn AsyncStream>>>,
}

impl AsyncStreamWithDatagram {
    pub fn new(stream: Box<dyn AsyncStream>) -> Self {
        AsyncStreamWithDatagram {
            stream: Arc::new(Mutex::new(stream)),
        }
    }

    pub async fn recv_datagram(&self, buffer: &mut [u8]) -> Result<usize, std::io::Error> {
        let mut stream = self.stream.lock().await;

        // First write the length of the datagram in u32, to the buffer
        let mut len_buffer = [0u8; 4];
        let len = stream.read_exact(&mut len_buffer).await?;
        if len != 4 {
            let msg = format!("recv datagram error: read len={}", len);
            error!("{}", msg);
            return Err(std::io::Error::new(std::io::ErrorKind::Other, msg));
        }

        let datagram_len = u32::from_be_bytes(len_buffer) as usize;
        if datagram_len > buffer.len() {
            let msg = format!(
                "recv datagram error with insufficient buffer: datagram_len={}, buffer_len={}",
                datagram_len,
                buffer.len()
            );
            error!("{}", msg);
            return Err(std::io::Error::new(std::io::ErrorKind::Other, msg));
        }

        let len = stream.read_exact(buffer[..datagram_len].as_mut()).await?;
        if len != datagram_len {
            let msg = format!(
                "recv datagram error: read len={}, expected len={}",
                len, datagram_len
            );
            error!("{}", msg);
            return Err(std::io::Error::new(std::io::ErrorKind::Other, msg));
        }

        Ok(len)
    }

    pub async fn send_datagram(&self, buffer: &[u8]) -> Result<usize, std::io::Error> {
        let mut stream = self.stream.lock().await;

        //TODO: u16 is enough?
        // First write the length of the datagram in u32, to the buffer
        let len = buffer.len() as u32;
        let len_buffer = len.to_be_bytes();
        stream.write_all(&len_buffer).await?;

        // Then write the datagram to the buffer
        stream.write_all(buffer).await?;

        Ok(len as usize)
    }
}

#[derive(Clone)]
pub struct RTcpTunnelDatagramClient {
    stream: AsyncStreamWithDatagram,
}

impl RTcpTunnelDatagramClient {
    pub fn new(stream: Box<dyn AsyncStream>) -> Self {
        Self {
            stream: AsyncStreamWithDatagram::new(stream),
        }
    }
}

#[async_trait::async_trait]
impl DatagramClient for RTcpTunnelDatagramClient {
    async fn recv_datagram(&self, buffer: &mut [u8]) -> Result<usize, std::io::Error> {
        self.stream.recv_datagram(buffer).await
    }

    async fn send_datagram(&self, buffer: &[u8]) -> Result<usize, std::io::Error> {
        self.stream.send_datagram(buffer).await
    }
}

#[derive(Clone)]
pub struct DatagramForwarder {
    target_addr: String,
    client: Arc<UdpSocket>,
    stream: AsyncStreamWithDatagram,
}

impl DatagramForwarder {
    pub async fn new(target_addr: &str, bind: &str, stream: Box<dyn AsyncStream>) -> std::io::Result<Self> {
        let client = UdpSocket::bind(bind).await.map_err(|e| {
            let msg = format!("UDP socket bind to {} failed: {:?}", bind, e);
            error!("{}", msg);
            std::io::Error::new(std::io::ErrorKind::Other, msg)
        })?;

        let ret = Self {
            target_addr: target_addr.to_string(),
            client: Arc::new(client),
            stream: AsyncStreamWithDatagram::new(stream),
        };

        Ok(ret)
    }

    pub fn start(&self) {
        let forwarder = self.clone();
        tokio::spawn(async move {
            match forwarder.run().await {
                Ok(_) => {
                    info!("Datagram forwarder stopped: {}", forwarder.target_addr);
                }
                Err(e) => {
                    error!(
                        "Datagram forwarder stopped with error: {}, {:?}",
                        forwarder.target_addr, e
                    );
                }
            }
        });
    }

    pub async fn run(&self) -> Result<(), std::io::Error> {
        let (recv, send) = tokio::join!(self.run_recv(), self.run_send());
        recv?;
        send?;

        Ok(())
    }

    async fn run_recv(&self) -> Result<(), std::io::Error> {
        loop {
            let mut buffer = [0u8; 1024 * 5];
            let (size, _) = self.client.recv_from(&mut buffer).await?;
            if size > 0 {
                let buffer = &buffer[..size];
                self.stream.send_datagram(buffer).await?;
            }
        }
    }

    async fn run_send(&self) -> Result<(), std::io::Error> {
        loop {
            let mut buffer = [0u8; 1024 * 5];
            let size = self.stream.recv_datagram(&mut buffer).await?;
            let buffer = &buffer[..size];
            self.client.send(buffer).await?;
        }
    }
}
