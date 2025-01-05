use super::package::*;
use super::protocol::*;
use super::stack::{WaitStream, NOTIFY_ROPEN_STREAM, WAIT_ROPEN_STREAM_MAP};
use crate::aes_stream::EncryptedStream;
use crate::tunnel::{DatagramClientBox, Tunnel, TunnelEndpoint};
use anyhow::Result;
use async_trait::async_trait;
use buckyos_kit::buckyos_get_unix_timestamp;
use buckyos_kit::AsyncStream;
use log::*;
use rand::Rng;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::task;
use tokio::time::timeout;

#[derive(Clone)]
pub(crate) struct RTcpTunnel {
    target: RTcpTarget,
    can_direct: bool,
    peer_addr: SocketAddr,
    this_device: String,
    aes_key: [u8; 32],
    //random_pk:[u8;32],
    //write_stream:Arc<Mutex<WriteHalf<EncryptedStream<TcpStream>>>>,
    //read_stream:Arc<Mutex<ReadHalf<EncryptedStream<TcpStream>>>>,
    write_stream: Arc<Mutex<WriteHalf<EncryptedStream<TcpStream>>>>,
    read_stream: Arc<Mutex<ReadHalf<EncryptedStream<TcpStream>>>>,

    next_seq: Arc<AtomicU32>,

    // Use to notify the open stream waiter
    open_resp_notify: Arc<Mutex<HashMap<u32, Arc<Notify>>>>,
}

impl RTcpTunnel {
    pub fn new(
        this_device: String,
        target: &RTcpTarget,
        can_direct: bool,
        stream: TcpStream,
        aes_key: [u8; 32],
        random_pk: [u8; 32],
    ) -> Self {
        let peer_addr = stream.peer_addr().unwrap();
        let mut iv = [0u8; 16];
        iv.copy_from_slice(&random_pk[..16]);
        let encrypted_stream = EncryptedStream::new(stream, &aes_key, &iv);
        let (read_stream, write_stream) = tokio::io::split(encrypted_stream);
        //let (read_stream,write_stream) =  tokio::io::split(stream);
        let mut this_target = target.clone();
        this_target.target_port = 0;
        RTcpTunnel {
            target: this_target,
            can_direct, //Considering the limit of port mapping, the default configuration is configured as "NoDirect" mode
            peer_addr: peer_addr,
            this_device: this_device,
            aes_key: aes_key,
            read_stream: Arc::new(Mutex::new(read_stream)),
            write_stream: Arc::new(Mutex::new(write_stream)),

            next_seq: Arc::new(AtomicU32::new(0)),
            open_resp_notify: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn close(&self) {
        //let mut read_stream = self.read_stream.lock().await;
        //let mut read_stream:OwnedReadHalf = (*read_stream);
        //read_stream.shutdown().await;
    }

    pub fn get_key(&self) -> &[u8; 32] {
        return &self.aes_key;
    }

    fn next_seq(&self) -> u32 {
        self.next_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    async fn process_package(&self, package: RTcpTunnelPackage) -> Result<(), anyhow::Error> {
        match package {
            RTcpTunnelPackage::Ping(ping_package) => {
                //send pong
                let pong_package = RTcpPongPackage::new(ping_package.seq, 0);
                let mut write_stream = self.write_stream.lock().await;
                let write_stream = Pin::new(&mut *write_stream);
                let _ = RTcpTunnelPackage::send_package(write_stream, pong_package).await?;
                return Ok(());
            }
            RTcpTunnelPackage::ROpen(ropen_package) => {
                // Get real request target address
                let request_target_addr = match ropen_package.body.dest_host {
                    Some(ref host) => format!("{}:{}", host, ropen_package.body.dest_port),
                    None => format!("127.0.0.1:{}", ropen_package.body.dest_port),
                };

                info!(
                    "rtcp tunnel ropen request target stream to {}",
                    request_target_addr
                );

                let nonce_bytes: [u8; 16] = hex::decode(ropen_package.body.stream_id.as_str())
                    .map_err(|op| anyhow::format_err!("decode stream_id error:{}", op))?
                    .try_into()
                    .map_err(|_op| anyhow::format_err!("decode stream_id error"))?;

                // 1. open stream to remote and send hello stream
                let mut target_addr = self.peer_addr.clone();
                target_addr.set_port(self.target.stack_port);
                let rtcp_stream = tokio::net::TcpStream::connect(target_addr).await;
                if rtcp_stream.is_err() {
                    error!(
                        "open rtcp stream to remote {} error:{}",
                        target_addr,
                        rtcp_stream.err().unwrap()
                    );
                    let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 2);
                    let mut write_stream = self.write_stream.lock().await;
                    let write_stream = Pin::new(&mut *write_stream);
                    RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;

                    return Ok(());
                }

                // 2. send ropen_resp
                {
                    let mut write_stream = self.write_stream.lock().await;
                    let write_stream = Pin::new(&mut *write_stream);
                    let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 0);
                    RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;
                }

                let mut rtcp_stream = rtcp_stream.unwrap();
                //let random_bytes: [u8; 16] =
                RTcpTunnelPackage::send_hello_stream(
                    &mut rtcp_stream,
                    ropen_package.body.stream_id.as_str(),
                )
                .await?;

                let aes_key = self.get_key().clone();
                let mut aes_stream = EncryptedStream::new(rtcp_stream, &aes_key, &nonce_bytes);

                // 3. First try to find if dispatcher exists for the target port
                let ret = super::dispatcher::RTCP_DISPATCHER_MANAGER
                    .get_dispatcher(ropen_package.body.dest_port);
                if let Some(dispatcher) = ret {
                    let end_point = TunnelEndpoint {
                        device_id: self.target.get_id_str(),
                        port: self.target.target_port,
                    };
                    dispatcher
                        .on_new_stream(Box::new(aes_stream), end_point)
                        .await?;
                    return Ok(());
                }

                // 4. If dispatcher does not exist, open a new stream to the real target
                let raw_stream_to_target =
                    tokio::net::TcpStream::connect(request_target_addr.clone()).await;
                if raw_stream_to_target.is_err() {
                    error!(
                        "open tcp stream to target {} error:{}",
                        request_target_addr,
                        raw_stream_to_target.err().unwrap()
                    );
                    let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 1);
                    let mut write_stream = self.write_stream.lock().await;
                    let write_stream = Pin::new(&mut *write_stream);
                    let _ =
                        RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;
                    return Ok(());
                }
                let mut raw_stream_to_target = raw_stream_to_target.unwrap();

                // 5. bind aes_stream and raw_stream_to_target
                task::spawn(async move {
                    info!(
                        "start copy aes_rtcp_stream to raw_tcp_stream,aes_key:{},nonce_bytes:{}",
                        hex::encode(aes_key),
                        hex::encode(nonce_bytes)
                    );
                    let copy_result =
                        tokio::io::copy_bidirectional(&mut aes_stream, &mut raw_stream_to_target)
                            .await;
                    if copy_result.is_err() {
                        error!(
                            "copy aes_rtcp_stream to raw_tcp_stream error:{}",
                            copy_result.err().unwrap()
                        );
                    } else {
                        let copy_len = copy_result.unwrap();
                        info!(
                            "copy aes_rtcp_stream to raw_tcp_stream ok,len:{:?}",
                            copy_len
                        );
                    }
                });

                return Ok(());
            }
            RTcpTunnelPackage::ROpenResp(_ropen_resp_package) => {
                //check result
                return Ok(());
            }
            RTcpTunnelPackage::Open(open_package) => {
                // Get real request target address
                let request_target_addr = match open_package.body.dest_host {
                    Some(ref host) => format!("{}:{}", host, open_package.body.dest_port),
                    None => format!("127.0.0.1:{}", open_package.body.dest_port),
                };

                info!(
                    "rtcp tunnel direct open target stream to {}",
                    request_target_addr
                );

                let mut raw_stream_to_target = None;
                // 1. First check if dispatcher exists for the target port
                let dispatcher = super::dispatcher::RTCP_DISPATCHER_MANAGER
                    .get_dispatcher(open_package.body.dest_port);
                if dispatcher.is_none() {
                    // 2. If dispatcher does not exist, open a new stream to the real target
                    let ret = tokio::net::TcpStream::connect(request_target_addr.clone()).await;
                    if ret.is_err() {
                        error!(
                            "open tcp stream to target {} error:{}",
                            request_target_addr,
                            ret.err().unwrap()
                        );
                        let open_resp_package = RTcpOpenRespPackage::new(open_package.seq, 1);
                        let mut write_stream = self.write_stream.lock().await;
                        let write_stream = Pin::new(&mut *write_stream);
                        RTcpTunnelPackage::send_package(write_stream, open_resp_package).await?;
                        return Ok(());
                    }

                    raw_stream_to_target = Some(ret.unwrap());
                }

                // 3. Prepare wait for the new stream before send open_resp
                let real_key = format!(
                    "{}_{}",
                    self.this_device.as_str(),
                    open_package.body.stream_id
                );
                WAIT_ROPEN_STREAM_MAP
                    .lock()
                    .await
                    .insert(real_key, WaitStream::Waiting);

                // 4. send open_resp with success
                {
                    let mut write_stream = self.write_stream.lock().await;
                    let write_stream = Pin::new(&mut *write_stream);
                    let open_resp_package = RTcpOpenRespPackage::new(open_package.seq, 0);
                    RTcpTunnelPackage::send_package(write_stream, open_resp_package).await?;
                }

                // 5. Wait for the new stream
                let stream = self.wait_ropen_stream(&open_package.body.stream_id).await?;

                let nonce_bytes: [u8; 16] = hex::decode(open_package.body.stream_id.as_str())
                    .map_err(|op| anyhow::format_err!("decode stream_id error:{}", op))?
                    .try_into()
                    .map_err(|_op| anyhow::format_err!("decode stream_id error"))?;
                let aes_key = self.get_key().clone();
                let mut aes_stream = EncryptedStream::new(stream, &aes_key, &nonce_bytes);

                // 6. If dispatcher exists, send the stream to the dispatcher
                if let Some(dispatcher) = dispatcher {
                    let end_point = TunnelEndpoint {
                        device_id: self.target.get_id_str(),
                        port: self.target.target_port,
                    };
                    dispatcher
                        .on_new_stream(Box::new(aes_stream), end_point)
                        .await?;
                    return Ok(());
                }

                assert!(raw_stream_to_target.is_some());
                let mut raw_stream_to_target = raw_stream_to_target.unwrap();

                // 7. bind aes_stream and raw_stream_to_target
                task::spawn(async move {
                    info!(
                        "start copy aes_rtcp_stream to raw_tcp_stream,aes_key:{},nonce_bytes:{}",
                        hex::encode(aes_key),
                        hex::encode(nonce_bytes)
                    );
                    let copy_result =
                        tokio::io::copy_bidirectional(&mut aes_stream, &mut raw_stream_to_target)
                            .await;
                    if copy_result.is_err() {
                        error!(
                            "copy aes_rtcp_stream to raw_tcp_stream error:{}",
                            copy_result.err().unwrap()
                        );
                    } else {
                        let copy_len = copy_result.unwrap();
                        info!(
                            "copy aes_rtcp_stream to raw_tcp_stream ok,len:{:?}",
                            copy_len
                        );
                    }
                });

                return Ok(());
            }
            RTcpTunnelPackage::OpenResp(open_resp_package) => {
                // Notify the open_stream waiter with the seq
                let notify = self
                    .open_resp_notify
                    .lock()
                    .await
                    .remove(&open_resp_package.seq);
                if notify.is_some() {
                    notify.unwrap().notify_one();
                } else {
                    warn!(
                        "open stream notify not found: seq={}",
                        open_resp_package.seq
                    );
                }

                return Ok(());
            }
            RTcpTunnelPackage::Pong(_pong_package) => {
                return Ok(());
            }
            t @ _ => {
                error!("Unsupport package type: {:?}", t);
                return Ok(());
            }
        }
    }

    pub async fn run(self) {
        let source_info = self.target.get_id_str();
        let mut read_stream = self.read_stream.lock().await;
        //let read_stream = self.read_stream.clone();
        loop {
            //等待超时 或 收到一个package
            //超时，基于last_active发送ping包,3倍超时时间后，关闭连接
            //收到一个package，处理package
            //   如果是req包，则处理逻辑后，发送resp包
            //   如果是resp包，则先找到对应的req包，然后处理逻辑

            let read_stream = Pin::new(&mut *read_stream);
            //info!("rtcp tunnel try read package from {}",self.peer_addr.to_string());

            let package =
                RTcpTunnelPackage::read_package(read_stream, false, source_info.as_str()).await;
            //info!("rtcp tunnel read package from {} ok",source_info.as_str());
            if package.is_err() {
                error!("read package error:{:?}", package.err().unwrap());
                break;
            }
            let package = package.unwrap();
            let result = self.process_package(package).await;
            if result.is_err() {
                error!("process package error:{}", result.err().unwrap());
                break;
            }
        }
    }

    async fn post_ropen(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
        session_key: &str,
    ) -> Result<(), std::io::Error> {
        let ropen_package = RTcpROpenPackage::new(
            self.next_seq(),
            session_key.to_string(),
            dest_port,
            dest_host,
        );
        let mut write_stream = self.write_stream.lock().await;
        let write_stream = Pin::new(&mut *write_stream);
        RTcpTunnelPackage::send_package(write_stream, ropen_package)
            .await
            .map_err(|e| {
                let msg = format!("send ropen package error:{}", e);
                error!("{}", msg);
                std::io::Error::new(std::io::ErrorKind::Other, msg)
            })
    }

    async fn post_open(
        &self,
        seq: u32,
        dest_port: u16,
        dest_host: Option<String>,
        session_key: &str,
    ) -> Result<(), std::io::Error> {
        let ropen_package =
            RTcpOpenPackage::new(seq, session_key.to_string(), dest_port, dest_host);
        let mut write_stream = self.write_stream.lock().await;
        let write_stream = Pin::new(&mut *write_stream);
        RTcpTunnelPackage::send_package(write_stream, ropen_package)
            .await
            .map_err(|e| {
                let msg = format!("send open package error:{}", e);
                error!("{}", msg);
                std::io::Error::new(std::io::ErrorKind::Other, msg)
            })
    }

    async fn wait_ropen_stream(&self, session_key: &str) -> Result<TcpStream, std::io::Error> {
        //let wait_map = WAIT_ROPEN_STREAM_MAP.clone();
        let wait_nofity = NOTIFY_ROPEN_STREAM.clone();
        let real_key = format!("{}_{}", self.this_device.as_str(), session_key);
        loop {
            let mut map = WAIT_ROPEN_STREAM_MAP.lock().await;
            let wait_stream = map.remove(real_key.as_str());

            if wait_stream.is_some() {
                match wait_stream.unwrap() {
                    WaitStream::OK(stream) => {
                        return Ok(stream);
                    }
                    WaitStream::Waiting => {
                        //do nothing
                        map.insert(real_key.clone(), WaitStream::Waiting);
                    }
                }
            }
            drop(map);
            if let Err(_) = timeout(Duration::from_secs(30), wait_nofity.notified()).await {
                warn!(
                    "Timeout: ropen stream {} was not found within the time limit.",
                    real_key.as_str()
                );
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Timeout"));
            }
        }
    }
}

#[async_trait]
impl Tunnel for RTcpTunnel {
    async fn ping(&self) -> Result<(), std::io::Error> {
        let timestamp = buckyos_get_unix_timestamp();
        let ping_package = RTcpPingPackage::new(0, timestamp);
        let mut write_stream = self.write_stream.lock().await;
        let write_stream = Pin::new(&mut *write_stream);
        let _ = RTcpTunnelPackage::send_package(write_stream, ping_package).await;
        Ok(())
    }

    async fn open_stream(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        // First generate 32byte session_key
        let random_bytes: [u8; 16] = rand::thread_rng().gen();
        let session_key = hex::encode(random_bytes);
        let real_key = format!("{}_{}", self.this_device.as_str(), session_key);

        if self.can_direct {
            let seq = self.next_seq();
            let notify = Arc::new(Notify::new());
            self.open_resp_notify
                .lock()
                .await
                .insert(seq, notify.clone());

            // Send open to target to build a direct stream
            self.post_open(seq, dest_port, dest_host, session_key.as_str())
                .await?;

            // Must wait openresp package then we can build a direct stream
            let wait_result = timeout(Duration::from_secs(60), notify.notified()).await;
            if wait_result.is_err() {
                self.open_resp_notify.lock().await.remove(&seq); // Remove the notify if timeout
                error!(
                    "Timeout: open stream {} was not found within the time limit.",
                    real_key.as_str()
                );
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Timeout"));
            }

            // Build a direct stream to target
            let mut target_addr = self.peer_addr.clone();
            target_addr.set_port(self.target.stack_port);
            let ret = tokio::net::TcpStream::connect(target_addr).await;
            if ret.is_err() {
                let e = ret.err().unwrap();
                error!(
                    "RTcp tunnel open direct stream to {}, {} error: {}",
                    target_addr,
                    self.target.get_id_str(),
                    e
                );
                return Err(e);
            }
            let mut stream = ret.unwrap();

            // Send hello stream
            RTcpTunnelPackage::send_hello_stream(&mut stream, session_key.as_str())
                .await
                .map_err(|e| {
                    let msg = format!("send hello stream error: {}, {}", target_addr, e);
                    error!("{}", msg);
                    std::io::Error::new(std::io::ErrorKind::Other, msg)
                })?;

            let aes_stream: EncryptedStream<TcpStream> =
                EncryptedStream::new(stream, &self.get_key(), &random_bytes);

            info!(
                "RTcp tunnel open direct stream to {}, {}",
                target_addr,
                self.target.get_id_str()
            );

            Ok(Box::new(aes_stream))
        } else {
            //send ropen to target

            WAIT_ROPEN_STREAM_MAP
                .lock()
                .await
                .insert(real_key.clone(), WaitStream::Waiting);
            //info!("insert session_key {} to wait ropen stream map",real_key.as_str());
            self.post_ropen(dest_port, dest_host, session_key.as_str())
                .await?;

            //wait new stream with session_key fomr target
            let stream = self.wait_ropen_stream(&session_key.as_str()).await?;
            let aes_stream: EncryptedStream<TcpStream> =
                EncryptedStream::new(stream, &self.get_key(), &random_bytes);
            //info!("wait ropen stream ok,return aes stream: aes_key:{},nonce_bytes:{}",hex::encode(self.get_key()),hex::encode(random_bytes));
            Ok(Box::new(aes_stream))
        }
    }

    async fn create_datagram_client(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        unimplemented!()
    }
}
