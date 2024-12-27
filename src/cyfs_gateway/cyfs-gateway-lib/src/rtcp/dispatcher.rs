use crate::tunnel::StreamListener;
use crate::tunnel::TunnelEndpoint;
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

pub struct RTcpDispatcherManager {
    dispatchers: Arc<Mutex<HashMap<u16, RTcpStreamDispatcher>>>,
}

impl RTcpDispatcherManager {
    pub fn new() -> RTcpDispatcherManager {
        RTcpDispatcherManager {
            dispatchers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn new_dispatcher(&self, bind_url: &Url) -> TunnelResult<RTcpStreamDispatcher> {
        let port = bind_url.port().unwrap_or(0);

        // Check if the dispatcher already exists
        let mut dispatchers = self.dispatchers.lock().unwrap();
        let dispatcher = dispatchers.get(&port);
        if dispatcher.is_some() {
            let msg = format!("RTcp dispatcher for port {} already exists", port);
            error!("{}", msg);
            return Err(TunnelError::AlreadyExists(msg));
        }

        info!("New RTcp dispatcher for url {}", bind_url);
        let dispatcher = RTcpStreamDispatcher::new(bind_url);
        dispatchers.insert(port, dispatcher.clone());
        Ok(dispatcher)
    }

    pub fn get_dispatcher(&self, port: u16) -> Option<RTcpStreamDispatcher> {
        let dispatchers = self.dispatchers.lock().unwrap();
        let dispatcher = dispatchers.get(&port);
        dispatcher.map(|d| d.clone())
    }
}

lazy_static::lazy_static! {
    pub static ref RTCP_DISPATCHER_MANAGER: RTcpDispatcherManager = RTcpDispatcherManager::new();
}
