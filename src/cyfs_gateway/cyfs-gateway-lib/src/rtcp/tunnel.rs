use super::package::*;
use super::protocol::*;
use super::stack::{WaitStream, NOTIFY_ROPEN_STREAM, WAIT_ROPEN_STREAM_MAP};
use crate::aes_stream::EncryptedStream;
use crate::tunnel::{DatagramClientBox, Tunnel};
use anyhow::Result;
use async_trait::async_trait;
use buckyos_kit::buckyos_get_unix_timestamp;
use buckyos_kit::AsyncStream;
use log::*;
use rand::Rng;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
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
}

impl RTcpTunnel {
    pub fn new(
        this_device: String,
        target: &RTcpTarget,
        _can_direct: bool,
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
            can_direct: false, //Considering the limit of port mapping, the default configuration is configured as "NoDirect" mode
            peer_addr: peer_addr,
            this_device: this_device,
            aes_key: aes_key,
            read_stream: Arc::new(Mutex::new(read_stream)),
            write_stream: Arc::new(Mutex::new(write_stream)),
            //random_pk:random_pk,
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
                //1.open stream2 to real target
                //TODO: will support connect to other ip
                let target_addr = match ropen_package.body.dest_host {
                    Some(ref host) => format!("{}:{}", host, ropen_package.body.dest_port),
                    None => format!("127.0.0.1:{}", ropen_package.body.dest_port),
                };

                info!(
                    "rtcp tunnel ropen: open real target stream to {}",
                    target_addr
                );
                let nonce_bytes: [u8; 16] = hex::decode(ropen_package.body.streamid.as_str())
                    .map_err(|op| anyhow::format_err!("decode streamid error:{}", op))?
                    .try_into()
                    .map_err(|_op| anyhow::format_err!("decode streamid error"))?;

                let raw_stream_to_target =
                    tokio::net::TcpStream::connect(target_addr.clone()).await;
                if raw_stream_to_target.is_err() {
                    error!(
                        "open raw tcp stream to target {} error:{}",
                        target_addr,
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
                //2.open stream to remote and send hello stream
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
                    let _ =
                        RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;
                    return Ok(());
                }

                //3. send ropen_resp
                let mut write_stream = self.write_stream.lock().await;
                let write_stream = Pin::new(&mut *write_stream);
                let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 0);
                RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;

                let mut rtcp_stream = rtcp_stream.unwrap();
                //let random_bytes: [u8; 16] =
                RTcpTunnelPackage::send_hello_stream(
                    &mut rtcp_stream,
                    ropen_package.body.streamid.as_str(),
                )
                .await?;
                let aes_key = self.get_key().clone();
                //4. 绑定两个stream
                task::spawn(async move {
                    let mut aes_stream = EncryptedStream::new(rtcp_stream, &aes_key, &nonce_bytes);
                    //info!("start copy aes_rtcp_stream to raw_tcp_stream,aes_key:{},nonce_bytes:{}",hex::encode(aes_key),hex::encode(nonce_bytes));
                    let _copy_result =
                        tokio::io::copy_bidirectional(&mut aes_stream, &mut raw_stream_to_target)
                            .await;
                    if _copy_result.is_err() {
                        error!(
                            "copy aes_rtcp_stream to raw_tcp_stream error:{}",
                            _copy_result.err().unwrap()
                        );
                    } else {
                        let copy_len = _copy_result.unwrap();
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

    async fn post_ropen(&self, dest_port: u16, dest_host: Option<String>, session_key: &str) {
        let ropen_package = RTcpROpenPackage::new(0, session_key.to_string(), dest_port, dest_host);
        let mut write_stream = self.write_stream.lock().await;
        let write_stream = Pin::new(&mut *write_stream);
        let _ = RTcpTunnelPackage::send_package(write_stream, ropen_package).await;
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
            if let Err(_) = timeout(Duration::from_secs(5), wait_nofity.notified()).await {
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
        if self.can_direct {
            let target = match dest_host {
                Some(host) => {
                    format!("{}:{}", host, dest_port)
                }
                None => {
                    format!("{}:{}", self.peer_addr.ip(), dest_port)
                }
            };

            info!(
                "RTcp tunnel open direct stream to {}#{}",
                target,
                self.target.get_id_str()
            );

            let stream = tokio::net::TcpStream::connect(&target).await?;
            Ok(Box::new(stream))
        } else {
            //send ropen to target
            //generate 32byte session_key
            let random_bytes: [u8; 16] = rand::thread_rng().gen();
            let session_key = hex::encode(random_bytes);
            let real_key = format!("{}_{}", self.this_device.as_str(), session_key);
            WAIT_ROPEN_STREAM_MAP
                .lock()
                .await
                .insert(real_key.clone(), WaitStream::Waiting);
            //info!("insert session_key {} to wait ropen stream map",real_key.as_str());
            self.post_ropen(dest_port, dest_host, session_key.as_str())
                .await;
            //wait new stream with session_key fomr target
            let stream = self.wait_ropen_stream(&session_key.as_str()).await?;
            let aes_stream = EncryptedStream::new(stream, &self.get_key(), &random_bytes);
            //info!("wait ropen stream ok,return aes stream: aes_key:{},nonce_bytes:{}",hex::encode(self.get_key()),hex::encode(random_bytes));
            Ok(Box::new(aes_stream))
        }
    }

    async fn create_datagram_client(
        &self,
        _dest_port: u16,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        unimplemented!()
    }
}
