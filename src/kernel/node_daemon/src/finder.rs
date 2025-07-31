//finder是一个服务，用来在局域网实现node之间的去中心互相发现
//尤其是多OOD体系启动的时候

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use std::collections::HashMap;
use std::sync::Arc;
use buckyos_kit::buckyos_get_unix_timestamp;
use tokio::sync::{Notify, RwLock};
use tokio::net::UdpSocket;
use tokio::time::timeout;
use serde::{Serialize, Deserialize};
use serde_json;
use log::*;

use anyhow::Result;
use jsonwebtoken::{decode, Validation, EncodingKey, DecodingKey, encode, Header, Algorithm};

const FINDER_SERVER_UDP_PORT: u16 = 2980;

#[derive(Serialize, Deserialize)]
pub struct LookingForReq {
    pub node_id: String,
    pub seq: u64,
    pub iam: String, //jwt
}

impl LookingForReq {
    pub fn new(node_id: String, seq: u64, iam: String) -> Self {
        Self { node_id, seq, iam }
    }

    pub fn decode_from_bytes(bytes: &[u8]) -> Result<Self> {
        let req = serde_json::from_slice(bytes)?;
        Ok(req)
    }

    pub fn encode_to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        Ok(bytes)
    }
}

#[derive(Serialize, Deserialize)]
pub struct LookingForResp {
    pub seq: u64,
    pub resp: String, //jwt
}

impl LookingForResp {
    pub fn new(seq: u64, resp: String) -> Self {
        Self { seq, resp }
    }

    pub fn decode_from_bytes(bytes: &[u8]) -> Result<Self> {
        let resp = serde_json::from_slice(bytes)?;
        Ok(resp)
    }

    pub fn encode_to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        Ok(bytes)
    }
}


pub struct NodeFinder {
    this_device_jwt: String,
    this_device_private_key: EncodingKey,
    running: Arc<RwLock<bool>>,
}

impl NodeFinder {
    pub fn new(this_device: String, device_private_key: EncodingKey) -> Self {
        Self {
            this_device_jwt: this_device,
            this_device_private_key: device_private_key,
            running: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn run_udp_server(&self) -> Result<()> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", FINDER_SERVER_UDP_PORT))
            .await.map_err(|e| { 
                warn!("bind NodeFinder server error: {}", e);
                anyhow::anyhow!("bind udp server error: {}", e)
            })?;

        let this_device_jwt = self.this_device_jwt.clone();
        let this_device_private_key = self.this_device_private_key.clone();
        let running = self.running.clone();
        tokio::spawn(async move {
            info!("NodeFinder server start.");
            let mut buf = [0; 4096];
            loop {
                let running = running.read().await;
                if !*running {
                    info!("Running is false, NodeFinder server will stop.");
                    break;
                }
                drop(running);
                let res = socket.recv_from(&mut buf).await;
                if res.is_err() {
                    warn!("recv from NodeFinder server error: {}", res.err().unwrap());
                    continue;
                }
                let (size, addr) = res.unwrap();
                let req = LookingForReq::decode_from_bytes(&buf[..size]);
                if req.is_ok() {
                    let req = req.unwrap();
                    let resp = LookingForResp::new(req.seq, this_device_jwt.clone());
                    let resp_bytes = resp.encode_to_bytes().unwrap();
                    let res = socket.send_to(&resp_bytes, addr).await;
                    if res.is_err() {
                        warn!("send to error: {}", res.err().unwrap());
                        continue;
                    }
                }
                else {
                    warn!("decode req error");
                }
            }
            info!("NodeFinder server stop.");
        });

        Ok(())
    }



    pub async fn stop_udp_server(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("NodeFinder server stop...");
    }
}

pub struct NodeFinderClient {
    this_device_jwt: String,
    this_device_private_key: EncodingKey,
}

impl NodeFinderClient {
    pub fn new(this_device_jwt: String, this_device_private_key: EncodingKey) -> Self {
        Self { this_device_jwt, this_device_private_key }
    }

    pub async fn looking_by_udpv4(&self, node_id: String, timeout_secs: u64) -> Result<IpAddr> {
        // 得到所有的ipv4地址
        // 在C类地址(局域网)上绑定udp端口
        // 每2秒发送UDP广播，内容为LookingForReq
        // 如果收到LookingForResp,则停止
        // 如果超时，则停止
        // 成功返回Resp的来源IP地址
        let broadcast_addrs = Self::get_ipv4_broadcast_addr().await?;
        let mut futures = Vec::new();
        let notify = Arc::new(Notify::new());
        
        
        for (ip,broadcast) in broadcast_addrs {
            let socket = UdpSocket::bind(format!("{}:0", ip)).await?;
            socket.set_broadcast(true)?;
            let to_address = format!("{}:{}", broadcast, FINDER_SERVER_UDP_PORT);

            let now = buckyos_get_unix_timestamp();
            let notify = notify.clone();
            let node_id = node_id.clone();
            let this_device_jwt = self.this_device_jwt.clone();
            let fut = async move {
                loop {
                    let req = LookingForReq::new(node_id.clone(), now, this_device_jwt.clone());
                    let req_bytes = req.encode_to_bytes().unwrap();
                    let res = socket.send_to(&req_bytes, to_address.clone()).await;
                    if res.is_err() {
                        warn!("send to {} error", to_address);
                        return;
                    }

                    let mut buf = [0; 4096];

                    tokio::select! {
                        res = socket.recv_from(&mut buf) => {
                            if res.is_ok() {
                                let (size, addr) = res.unwrap();
                                debug!("recv from {}", to_address);
                            }
                            else {
                                warn!("recv from {} error", to_address);
                            }        
                        }
                        _ = notify.notified() => {
                            // 有其他任务已完成，退出
                            debug!("notify");
                        }
                    }
                }
            };
            futures.push(fut);
        }

        let _ = futures::future::join_all(futures).await;
        unimplemented!();
    }

    async fn get_ipv4_broadcast_addr() -> Result<Vec<(Ipv4Addr,Ipv4Addr)>> {
        let interfaces = if_addrs::get_if_addrs().unwrap();
        let mut broadcast_addrs = Vec::new();
        for interface in interfaces {
            if interface.is_loopback() {
                continue;
            }

            match interface.addr {
                if_addrs::IfAddr::V4(ifv4addr) => {
                    let ip = ifv4addr.ip;
                    let netmask = ifv4addr.netmask;
                    let broadcast = ifv4addr.broadcast;
                    if broadcast.is_some() {
                        let broadcast = broadcast.unwrap();
                        //debug!("ip: {:?}, netmask: {:?}, broadcast: {:?}", ip, netmask, broadcast);
                        broadcast_addrs.push((ip,broadcast));
                    }
                }
                _ => {
                    continue;
                }
            }
        }
        return Ok(broadcast_addrs);
    }  
    
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use if_addrs;
    #[tokio::test]
    async fn test_get_all_local_ipv4_addresses() {
        let ips = NodeFinderClient::get_ipv4_broadcast_addr().await.unwrap();
        for ip in ips {
            println!("ip: {:?}, broadcast: {:?}", ip.0, ip.1);
        }
    }
    #[tokio::test]
    async fn test_run_udp_server() {
        buckyos_kit::init_logging("test_node_finder", false);
        let device_private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
        "#;
        let device_private_key = EncodingKey::from_ed_pem(device_private_key_pem.as_bytes()).unwrap();
        let finder = NodeFinder::new("test".to_string(), device_private_key);
        finder.run_udp_server().await.unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
        finder.stop_udp_server().await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

}
