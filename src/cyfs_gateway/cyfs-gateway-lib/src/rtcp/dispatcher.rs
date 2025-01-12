use super::datagram::AsyncStreamWithDatagram;
use crate::tunnel::TunnelEndpoint;
use crate::tunnel::{DatagramServer, StreamListener};
use crate::{TunnelError, TunnelResult};
use buckyos_kit::AsyncStream;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;
use url::Url;

#[derive(Clone)]
pub struct RTcpStreamDispatcher {
    bind_url: Url,
    rx: Arc<AsyncMutex<mpsc::Receiver<(Box<dyn AsyncStream>, TunnelEndpoint)>>>,
    tx: Arc<AsyncMutex<mpsc::Sender<(Box<dyn AsyncStream>, TunnelEndpoint)>>>,
}

impl RTcpStreamDispatcher {
    pub fn new(bind_url: &Url) -> RTcpStreamDispatcher {
        let (tx, rx) = mpsc::channel(100);

        Self {
            bind_url: bind_url.clone(),
            rx: Arc::new(AsyncMutex::new(rx)),
            tx: Arc::new(AsyncMutex::new(tx)),
        }
    }

    pub async fn on_new_stream(
        &self,
        stream: Box<dyn AsyncStream>,
        endpoint: TunnelEndpoint,
    ) -> TunnelResult<()> {
        let tx = self.tx.lock().await;
        tx.send((stream, endpoint)).await.map_err(|e| {
            let msg = format!("send new stream to dispatcher failed: {:?}", e);
            error!("{}", msg);
            TunnelError::IoError(msg)
        })
    }
}

#[async_trait::async_trait]
impl StreamListener for RTcpStreamDispatcher {
    async fn accept(&self) -> Result<(Box<dyn AsyncStream>, TunnelEndpoint), std::io::Error> {
        match self.rx.lock().await.recv().await {
            Some((stream, endpoint)) => Ok((stream, endpoint)),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "accept error",
            )),
        }
    }
}

#[derive(Clone)]
pub struct RTcpDatagramDispatcher {
    bind_url: Url,
    streams: Arc<Mutex<HashMap<TunnelEndpoint, AsyncStreamWithDatagram>>>,

    rx: Arc<AsyncMutex<mpsc::Receiver<(Vec<u8>, TunnelEndpoint)>>>,
    tx: Arc<AsyncMutex<mpsc::Sender<(Vec<u8>, TunnelEndpoint)>>>,
}

impl RTcpDatagramDispatcher {
    pub fn new(bind_url: &Url) -> RTcpDatagramDispatcher {
        let (tx, rx) = mpsc::channel(100);

        RTcpDatagramDispatcher {
            bind_url: bind_url.clone(),
            streams: Arc::new(Mutex::new(HashMap::new())),
            rx: Arc::new(AsyncMutex::new(rx)),
            tx: Arc::new(AsyncMutex::new(tx)),
        }
    }

    pub async fn on_new_stream(
        &self,
        stream: Box<dyn AsyncStream>,
        endpoint: TunnelEndpoint,
    ) -> TunnelResult<()> {
        let mut streams = self.streams.lock().unwrap();
        let stream = AsyncStreamWithDatagram::new(stream);
        let prev = streams.insert(endpoint.clone(), stream.clone());
        if prev.is_some() {
            // If the stream already exists, we should replace it
            let msg = format!(
                "RTcp stream for endpoint {:?} already exists, now will replace",
                endpoint
            );
            warn!("{}", msg);
        }

        // Start recv from the stream and send to the dispatcher
        let this = self.clone();
        tokio::spawn(async move {
            this.run_recv(stream, endpoint).await;
        });

        Ok(())
    }

    async fn run_recv(&self, stream: AsyncStreamWithDatagram, endpoint: TunnelEndpoint) {
        let mut buffer = vec![0u8; 4096];
        loop {
            match stream.recv_datagram(&mut buffer).await {
                Ok(len) => {
                    if len == 0 {
                        let msg =
                            format!("recv datagram from endpoint {:?} with 0 length", endpoint);
                        warn!("{}", msg);
                        continue;
                    }

                    let datagram = buffer[..len].to_vec();
                    let tx = self.tx.lock().await;
                    if let Err(e) = tx.send((datagram, endpoint.clone())).await {
                        let msg = format!("Send datagram to dispatcher failed: {:?}", e);
                        error!("{}", msg);
                        break;
                    }
                }
                Err(e) => {
                    let msg = format!("recv datagram from endpoint {:?} failed: {:?}", endpoint, e);
                    error!("{}", msg);

                    // Remove the stream from the streams map
                    let mut streams = self.streams.lock().unwrap();
                    streams.remove(&endpoint);
                    break;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl DatagramServer for RTcpDatagramDispatcher {
    async fn recv_datagram(
        &self,
        buffer: &mut [u8],
    ) -> Result<(usize, TunnelEndpoint), std::io::Error> {
        match self.rx.lock().await.recv().await {
            Some((datagram, ep)) => {
                let len = datagram.len();
                if buffer.len() < len {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "buffer size not enough",
                    ));
                }
                
                buffer[..len].copy_from_slice(&datagram);
                Ok((len, ep))
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "recv datagram error",
            )),
        }
    }

    async fn send_datagram(
        &self,
        ep: &TunnelEndpoint,
        buffer: &[u8],
    ) -> Result<usize, std::io::Error> {
        let stream = {
            let streams = self.streams.lock().unwrap();
            let stream = streams.get(ep);
            if stream.is_none() {
                let msg = format!("RTcp stream for endpoint {:?} not found", ep);
                error!("{}", msg);
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, msg));
            }

            stream.unwrap().to_owned()
        };

        match stream.send_datagram(buffer).await {
            Ok(len) => Ok(len),
            Err(e) => {
                let msg = format!("send datagram to endpoint {:?} failed: {:?}", ep, e);
                error!("{}", msg);

                // Remove the stream from the streams map
                let mut streams = self.streams.lock().unwrap();
                streams.remove(ep);
                Err(e)
            }
        }
    }
}

pub struct RTcpDispatcherManager {
    stream_dispatchers: Arc<Mutex<HashMap<u16, RTcpStreamDispatcher>>>,
    datagram_dispatchers: Arc<Mutex<HashMap<u16, RTcpDatagramDispatcher>>>,
}

impl RTcpDispatcherManager {
    pub fn new() -> RTcpDispatcherManager {
        Self {
            stream_dispatchers: Arc::new(Mutex::new(HashMap::new())),
            datagram_dispatchers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn new_stream_dispatcher(&self, bind_url: &Url) -> TunnelResult<RTcpStreamDispatcher> {
        let port = bind_url.port().unwrap_or(0);

        // FIXME check if port is 0? what does it mean?

        // Check if the dispatcher already exists
        let mut dispatchers = self.stream_dispatchers.lock().unwrap();
        let dispatcher = dispatchers.get(&port);
        if dispatcher.is_some() {
            let msg = format!("RTcp stream dispatcher for port {} already exists", port);
            error!("{}", msg);
            return Err(TunnelError::AlreadyExists(msg));
        }

        info!("New RTcp dispatcher for url {}", bind_url);
        let dispatcher = RTcpStreamDispatcher::new(bind_url);
        dispatchers.insert(port, dispatcher.clone());
        Ok(dispatcher)
    }

    pub fn new_datagram_dispatcher(&self, bind_url: &Url) -> TunnelResult<RTcpDatagramDispatcher> {
        let port = bind_url.port().unwrap_or(0);

        // Check if the dispatcher already exists
        let mut dispatchers = self.datagram_dispatchers.lock().unwrap();
        let dispatcher = dispatchers.get(&port);
        if dispatcher.is_some() {
            let msg = format!("RTcp stream dispatcher for port {} already exists", port);
            error!("{}", msg);
            return Err(TunnelError::AlreadyExists(msg));
        }

        info!("New RTcp dispatcher for url {}", bind_url);
        let dispatcher = RTcpDatagramDispatcher::new(bind_url);
        dispatchers.insert(port, dispatcher.clone());
        Ok(dispatcher)
    }

    pub fn get_stream_dispatcher(&self, port: u16) -> Option<RTcpStreamDispatcher> {
        let dispatchers = self.stream_dispatchers.lock().unwrap();
        let dispatcher = dispatchers.get(&port);
        dispatcher.map(|d| d.clone())
    }

    pub fn get_datagram_dispatcher(&self, port: u16) -> Option<RTcpDatagramDispatcher> {
        let dispatchers = self.datagram_dispatchers.lock().unwrap();
        let dispatcher = dispatchers.get(&port);
        dispatcher.map(|d| d.clone())
    }
}

lazy_static::lazy_static! {
    pub static ref RTCP_DISPATCHER_MANAGER: RTcpDispatcherManager = RTcpDispatcherManager::new();
}
