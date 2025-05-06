use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify};
use tokio::time::{timeout, Duration};
use tokio::io::AsyncWriteExt;


pub(crate) enum WaitStream {
    Waiting,
    OK(TcpStream),
}

impl WaitStream {
    fn unwrap(self) -> TcpStream {
        match self {
            WaitStream::OK(stream) => stream,
            _ => panic!("unwrap WaitStream error"),
        }
    }
}

#[derive(Clone)]
pub struct RTcpStreamBuildHelper {
    notify_ropen_stream: Arc<Notify>,
    wait_ropen_stream_map: Arc<Mutex<HashMap<String, WaitStream>>>,
}

impl RTcpStreamBuildHelper {
    pub fn new() -> Self {
        RTcpStreamBuildHelper {
            notify_ropen_stream: Arc::new(Notify::new()),
            wait_ropen_stream_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn notify_ropen_stream(&self, mut stream: TcpStream, key: &str) {
        let mut wait_streams= self.wait_ropen_stream_map.lock().await;
        let wait_session = wait_streams.get_mut(key);
        if wait_session.is_none() {
            let clone_map: Vec<String> = wait_streams.keys().cloned().collect();
            error!("No wait session for {}, map is {:?}", key, clone_map);

            let _ = stream.shutdown().await;

            return;
        }

        // bind stream to session and notify
        let wait_session = wait_session.unwrap();
        *wait_session = WaitStream::OK(stream);

        self.notify_ropen_stream.notify_waiters();
    }

    pub async fn new_wait_stream(&self, key: &str) {
        let mut map = self.wait_ropen_stream_map.lock().await;
        if let Some(_ret) = map.insert(key.to_string(), WaitStream::Waiting) {
            // FIXME: should we return error here?
            error!("new_wait_stream: key {} already exists", key);
        }
    }

    pub async fn wait_ropen_stream(&self, key: &str) -> Result<TcpStream, std::io::Error> {
        loop {
            let mut map = self.wait_ropen_stream_map.lock().await;
            let wait_stream = map.remove(key);

            if wait_stream.is_some() {
                match wait_stream.unwrap() {
                    WaitStream::OK(stream) => {
                        return Ok(stream);
                    }
                    WaitStream::Waiting => {
                        // do nothing
                        map.insert(key.to_owned(), WaitStream::Waiting);
                    }
                }
            }
            drop(map);

            if let Err(_) =
                timeout(Duration::from_secs(30), self.notify_ropen_stream.notified()).await
            {
                warn!(
                    "Timeout: ropen stream {} was not found within the time limit.",
                    key
                );
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Timeout"));
            }
        }
    }
}