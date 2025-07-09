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
        let timeout_duration = Duration::from_secs(30);
        let start_time = std::time::Instant::now();
        
        loop {
            // 检查是否超时
            if start_time.elapsed() >= timeout_duration {
                warn!(
                    "Timeout: ropen stream {} was not found within the time limit.",
                    key
                );
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Timeout"));
            }
            
            // 检查 map 中是否有对应的 stream
            {
                let mut map = self.wait_ropen_stream_map.lock().await;
                if let Some(wait_stream) = map.remove(key) {
                    match wait_stream {
                        WaitStream::OK(stream) => {
                            return Ok(stream);
                        }
                        WaitStream::Waiting => {
                            // 重新插入等待状态，继续等待
                            map.insert(key.to_owned(), WaitStream::Waiting);
                        }
                    }
                }
            }
            
            // 等待一小段时间后再次检查，避免过度占用 CPU
            let remaining_time = timeout_duration - start_time.elapsed();
            let check_interval = std::cmp::min(Duration::from_millis(100), remaining_time);
            
            if let Err(_) = timeout(check_interval, self.notify_ropen_stream.notified()).await {
                // 超时继续循环检查
                continue;
            }
        }
    }
}