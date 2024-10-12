#![allow(unused)]
use std::pin::Pin;
/*
tunnel的控制协议
二进制头：2+1+4=7字节
len: u16,如果是0则表示该Package是HelloStream包，后面是32字节的session_key
json_pos:u8, json数据的起始位置
cmd:u8,命令类型
seq:u32, 序列号
package:String， data in json format

控制协议分为如下类型：

// 建立tunnel，tunnel建立后，client需要立刻发送build包，用以确定该tunnel的信息
{
cmd:hello
from_id: string,
to_id: string,
test_port:u16
seession_key:option<string>
}

{
cmd:hello_ack
test_result:bool
}


{
cmd:ping

}
{
cmd:ping_resp
}


//因为无法与对端建立直连，因此通过该命令，要求对方建立反连，效果相当于命令发起方主动连接target
//并不使用直接复用当前tunnel+rebuild的方式,是想提高一些扩展性
//要求对端返连自己的端口
{
cmd:ropen
session_key:string （32个字节的随机字符串，第1，2个字符是byte 0）
target:Url,like tcp://_:123
}

{
cmd:ropen_resp
result:u32
}

*/
use std::time::Duration;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::net::tcp::ReadHalf;
use tokio::stream;
use tokio::sync::{Mutex, Notify};
use tokio::task;
use tokio::time::timeout;
use url::form_urlencoded::ByteSerialize;
use std::collections::HashMap;
use std::net::{SocketAddr};
use std::sync::Arc;
use std::{fmt::Debug, net::IpAddr};
use std::str::FromStr;
use tokio::net::{TcpListener,TcpStream};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use log::*;
use async_trait::async_trait;
use url::Url;
use lazy_static::lazy_static;
use anyhow::Result;
use crate::{ tunnel, TunnelEndpoint, TunnelError, TunnelResult};
use crate::tunnel::{AsyncStream, DatagramClientBox, DatagramServerBox, StreamListener, Tunnel, TunnelBox, TunnelBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmdType {
    UnknowProtocol = 0,
    Hello = 1,
    HelloAck = 2,
    Ping = 3,
    Pong = 4,
    ROpen = 5,
    ROpenResp = 6,
}

impl From<u8> for CmdType {
    fn from(value: u8) -> Self {
        match value {
            1 => CmdType::Hello,
            2 => CmdType::HelloAck,
            3 => CmdType::Ping,
            4 => CmdType::Pong,
            5 => CmdType::ROpen,
            6 => CmdType::ROpenResp,
            _ => CmdType::UnknowProtocol,
        }
    }
}

impl Into<u8> for CmdType {
    fn into(self) -> u8 {
        self as u8
    }
}

#[derive(Serialize, Deserialize, Debug,Clone)]
struct RTcpTunnelPackageImpl<T> 
    where T: Serialize 
{
    len: u16,
    json_pos: u8,
    cmd:u8,
    seq: u32,
    body: T,
}

#[derive(Serialize, Deserialize, Debug,Clone)]
struct RTcpHelloBody {
    from_id: String,
    to_id: String,
    my_port: u16,
    session_key: Option<String>,
}
type RTcpHelloPackage = RTcpTunnelPackageImpl<RTcpHelloBody>;
impl RTcpHelloPackage {
    pub fn new(seq:u32,from_id: String, to_id: String, my_port: u16, session_key: Option<String>) -> Self {
        RTcpHelloPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Hello.into(),
            seq: seq,
            body: RTcpHelloBody {
                from_id,
                to_id,
                my_port,
                session_key,
            },
        }
    }

    pub fn from_json(seq: u32,json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpHelloBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"parse package error"));
        }
        let pakcage = RTcpHelloPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Hello.into(),
            seq: seq,
            body: body.unwrap(),
        };
        Ok(pakcage)
    }
}

#[derive(Serialize, Deserialize, Debug,Clone)]
struct RTcpHelloAckBody {
    test_result: bool,
}
type RTcpHelloAckPackage = RTcpTunnelPackageImpl<RTcpHelloAckBody>;
impl RTcpHelloAckPackage {
    pub fn new(seq:u32,test_result: bool) -> Self {
        RTcpHelloAckPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::HelloAck.into(),
            seq: seq,
            body: RTcpHelloAckBody {
                test_result,
            },
        }
    }

    pub fn from_json(seq: u32,json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpHelloAckBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"parse package error"));
        }
        let package = RTcpHelloAckPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::HelloAck.into(),
            seq: seq,
            body: body.unwrap(),
        };
        Ok(package)
    }
}

#[derive(Serialize, Deserialize, Debug,Clone)]
struct RTcpPingBody {
    timestamp: u64,
}
type RTcpPingPackage = RTcpTunnelPackageImpl<RTcpPingBody>;
impl RTcpPingPackage {
    pub fn new(seq:u32,timestamp: u64) -> Self {
        RTcpPingPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Ping.into(),
            seq: seq,
            body: RTcpPingBody {
                timestamp,
            },
        }
    }

    pub fn from_json(seq: u32,json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpPingBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"parse package error"));
        }
        let package = RTcpPingPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Ping.into(),
            seq: seq,
            body: body.unwrap(),
        };
        Ok(package)
    }
}

#[derive(Serialize, Deserialize, Debug,Clone)]
struct RTcpPongBody {
    timestamp: u64,
}
type RTcpPongPackage = RTcpTunnelPackageImpl<RTcpPongBody>;
impl RTcpPongPackage {
    pub fn new(seq:u32,timestamp: u64) -> Self {
        RTcpPongPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Pong.into(),
            seq: seq,
            body: RTcpPongBody {
                timestamp,
            },
        }
    }

    pub fn from_json(seq: u32,json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpPongBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"parse package error"));
        }

        let package = RTcpPongPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Pong.into(),
            seq: seq,
            body: body.unwrap(),
        };
        
        Ok(package)
    }
}

#[derive(Serialize, Deserialize, Debug,Clone)]
struct RTcpROpenBody {
    session_key: String,
    dest_port: u16,
}
type RTcpROpenPackage = RTcpTunnelPackageImpl<RTcpROpenBody>;

impl RTcpROpenPackage {
    pub fn new(seq:u32,session_key: String, dest_port: u16) -> Self {
        RTcpROpenPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::ROpen.into(),
            seq: seq,
            body: RTcpROpenBody {
                session_key,
                dest_port,
            },
        }
    }

    pub fn from_json(seq: u32,json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpROpenBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"parse package error"));
        }
        
        let package = RTcpROpenPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::ROpen.into(),
            seq: seq,
            body: body.unwrap(),
        };
        Ok(package)
    }
}

#[derive(Serialize, Deserialize, Debug,Clone)]
struct RTcpROpenRespBody {
    result: u32,
}
type RTcpROpenRespPackage = RTcpTunnelPackageImpl<RTcpROpenRespBody>;
impl RTcpROpenRespPackage {
    pub fn new(seq:u32,result: u32) -> Self {
        RTcpROpenRespPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::ROpenResp.into(),
            seq: seq,
            body: RTcpROpenRespBody {
                result,
            },
        }
    }

    pub fn from_json(seq: u32,json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpROpenRespBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"parse package error"));
        }
        
        let package = RTcpROpenRespPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::ROpenResp.into(),
            seq: seq,
            body: body.unwrap(),
        };
        Ok(package)
    }
}

#[derive(Clone)]
enum RTcpTunnelPackage {
    HelloStream(String),
    Hello(RTcpHelloPackage),
    HelloAck(RTcpHelloAckPackage),
    Ping(RTcpPingPackage),
    Pong(RTcpPongPackage),
    ROpen(RTcpROpenPackage),
    ROpenResp(RTcpROpenRespPackage),
}

impl RTcpTunnelPackage {
    pub async fn read_package<'a,S>(mut stream:Pin<&'a mut S>,is_first_package:bool) -> Result<RTcpTunnelPackage,std::io::Error> 
        where S: AsyncReadExt + 'a,
    {
        let mut buf = [0; 2];
        stream.read_exact(&mut buf).await?;
        let len = u16::from_be_bytes(buf);
        if len == 0 {
            if !is_first_package {
                error!("HelloStream MUST be first package.");
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"first package is not HelloStream"));
            }

            let mut buf = [0; 32];
            stream.read_exact(&mut buf).await?;
            let session_key = String::from_utf8_lossy(&buf);
            return Ok(RTcpTunnelPackage::HelloStream(session_key.to_string()));
        } else {
            let _len = len - 2;
            let mut buf = vec![0; _len as usize];
            stream.read_exact(&mut buf).await?;

            let mut pos = 0;
            let json_pos = buf[pos];
            if json_pos < 6 {
                error!("json_pos is invalid");
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"json_pos is invalid"));
            }
            pos = pos + 1;

            let read_buf = &buf[pos..pos+1];
            let cmd_type = read_buf[0];
            pos = pos + 1;

            let read_buf = &buf[pos..pos+4];
            let seq = u32::from_be_bytes(read_buf.try_into().unwrap());
            pos += 4;

            //start read json
            let _len = json_pos -2;
            let mut read_buf = &buf[(_len as usize)..];
            let json_str: std::borrow::Cow<'_, str> = String::from_utf8_lossy(read_buf);
            let package_value = serde_json::from_str(json_str.as_ref());
            if package_value.is_err() {
                error!("parse package error:{}",package_value.err().unwrap());
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"parse package error"));
            }
            let package_value = package_value.unwrap();

            let cmd = CmdType::from(cmd_type);
            match cmd {
                CmdType::Hello => {
                    let result_package = RTcpHelloPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::Hello(result_package));
                },
                CmdType::HelloAck => {
                    let result_package = RTcpHelloAckPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::HelloAck(result_package));
                },
                CmdType::Ping => {
                    let result_package: RTcpTunnelPackageImpl<RTcpPingBody> = RTcpPingPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::Ping(result_package));
                },
                CmdType::Pong => {
                    let result_package = RTcpPongPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::Pong(result_package));
                },
                CmdType::ROpen => {
                    let result_package = RTcpROpenPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::ROpen(result_package));
                },
                CmdType::ROpenResp => {
                    let result_package = RTcpROpenRespPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::ROpenResp(result_package));
                },
                _ => {
                    error!("unsupport package type");
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"unsupport package type"));
                }
            }
        }
        let n = stream.read(&mut buf).await?;
        let str = String::from_utf8_lossy(&buf[..n]);
        unimplemented!()
    }

    pub async fn send_package<S,T>(mut stream:Pin<&mut S>,pkg:RTcpTunnelPackageImpl<T>) -> Result<()> 
        where 
            T: Serialize,
            S: AsyncWriteExt   
    {
        //encode package to json
        let json_str = serde_json::to_string(&pkg.body).unwrap();
        let json_pos:u8 = 2 + 1 + 1 + 4;
        let total_len = 2 + 1 + 1 + 4 + json_str.len();
        if total_len > 0xffff {
            error!("package too long");
            return Err(anyhow::format_err!("package too long"));
        }

        let mut write_buf: Vec<u8> = Vec::new();
        let bytes = u16::to_be_bytes(total_len as u16);
        write_buf.extend_from_slice(&bytes);
        write_buf.extend(std::iter::once(json_pos));
        write_buf.extend(std::iter::once(pkg.cmd as u8));
        let bytes = u32::to_be_bytes(pkg.seq);
        write_buf.extend_from_slice(&bytes);
        write_buf.extend_from_slice(json_str.as_bytes());
        stream.write_all(&write_buf).await?;
        Ok(())
    }

    pub async fn send_hello_stream(stream:&mut TcpStream,session_key:&str) -> Result<(),anyhow::Error> {
        let total_len = 0;
        let mut write_buf: Vec<u8> = Vec::new();
        let bytes = u16::to_be_bytes(total_len);
        write_buf.extend_from_slice(&bytes);
        write_buf.extend_from_slice(session_key.as_bytes());
        stream.write_all(&write_buf).await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct RTcpTunnel {
    target: Url,
    can_direct: bool,
    peer_addr : SocketAddr,
    
    write_stream:Arc<Mutex<OwnedWriteHalf>>,
    read_stream:Arc<Mutex<OwnedReadHalf>>,
}

impl RTcpTunnel {
    pub fn new(target: Url,  can_direct: bool,stream:TcpStream) -> Self {
        let peer_addr = stream.peer_addr().unwrap();    
        let (read_stream,write_stream) = stream.into_split();
        RTcpTunnel {
            target,
            can_direct,
            peer_addr: peer_addr,
            read_stream : Arc::new(Mutex::new(read_stream)),
            write_stream : Arc::new(Mutex::new(write_stream)),
        }
    }

    pub fn close(&self) {
        unimplemented!("close not implemented");
    }

    async fn process_package(&self,package:RTcpTunnelPackage) -> Result<(),anyhow::Error> {
        match package {
            RTcpTunnelPackage::Ping(ping_package) => {
                //send pong
                let pong_package = RTcpPongPackage::new(ping_package.seq,0);
                let mut write_stream = self.write_stream.lock().await;
                let mut write_stream = Pin::new(&mut *write_stream);
                let _ = RTcpTunnelPackage::send_package(write_stream,pong_package).await?;
                return Ok(());
            },
            RTcpTunnelPackage::ROpen(ropen_package) => {
                //open stream1 to target
                if !self.can_direct {
                    warn!("tunnel can not direct, ignore ropen"); 
                    return Ok(());
                }

                //1.open stream2 to real target
                //TODO: will support connect to other ip
                let target_addr = format!("127.0.0.1:{}",ropen_package.body.dest_port);
                let mut stream1 = tokio::net::TcpStream::connect(target_addr.clone()).await;
                if stream1.is_err() {
                    error!("open stream to target {} error:{}",target_addr,stream1.err().unwrap());
                    let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 1); 
                    let mut write_stream = self.write_stream.lock().await;
                    let mut write_stream = Pin::new(&mut *write_stream);
                    let _ = RTcpTunnelPackage::send_package(write_stream,ropen_resp_package).await?;
                    return Ok(());
                }
                let mut stream1 = stream1.unwrap();
                //2.open stream to remote and send hello stream
                let mut stream2 = tokio::net::TcpStream::connect(self.peer_addr).await;
                if stream2.is_err() {
                    error!("open stream to remote {} error:{}",self.peer_addr,stream2.err().unwrap());
                    let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 2);
                    let mut write_stream = self.write_stream.lock().await;
                    let mut write_stream = Pin::new(&mut *write_stream);
                    let _ = RTcpTunnelPackage::send_package(write_stream,ropen_resp_package).await?;
                    return Ok(());
                }

                let mut stream2 = stream2.unwrap();
                RTcpTunnelPackage::send_hello_stream(&mut stream2,ropen_package.body.session_key.as_str()).await?;
                //3. 绑定两个stream
                task::spawn(async move {
                    let _copy_result = tokio::io::copy_bidirectional(&mut stream2,&mut stream1).await;
                    if _copy_result.is_err() {
                        error!("copy stream2 to stream1 error:{}",_copy_result.err().unwrap());
                    }
                });

                //4. send ropen_resp
                let mut write_stream = self.write_stream.lock().await;
                let mut write_stream = Pin::new(&mut *write_stream);
                let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 0);
                RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;
                return Ok(());
            },
            RTcpTunnelPackage::ROpenResp(ropen_resp_package) => {
                //check result
                return Ok(());
            },
            _ => {
                error!("unsupport package type");
                return Ok(());
            }
        }
        return Ok(());
    }

    async fn run(self) {
        loop {
            //等待超时 或 收到一个package
            //超时，基于last_active发送ping包,3倍超时时间后，关闭连接
            //收到一个package，处理package
            //   如果是req包，则处理逻辑后，发送resp包
            //   如果是resp包，则先找到对应的req包，然后处理逻辑
            let read_stream = self.read_stream.clone();
            let mut read_stream = read_stream.lock().await;
            let mut read_stream = Pin::new(&mut *read_stream);
            let package = RTcpTunnelPackage::read_package(read_stream,false).await;
            if package.is_err() {
                error!("read package error:{}",package.err().unwrap());
                break;
            }
            let package = package.unwrap();
            let result = self.process_package(package).await;
            if result.is_err() {
                error!("process package error:{}",result.err().unwrap());
                break;
            }
        }
    }

    async fn post_ropen(&self,dest_port:u16,session_key:&str) {
        let ropen_package = RTcpROpenPackage::new(0,session_key.to_string(),dest_port);
        let mut write_stream = self.write_stream.lock().await;
        let mut write_stream = Pin::new(&mut *write_stream);
        let _ = RTcpTunnelPackage::send_package(write_stream,ropen_package).await;
    }

    async fn wait_ropen_stream(&self,session_key:&str) -> Result<TcpStream,std::io::Error> {
        let wait_map = WAIT_ROPEN_STREAM_MAP.clone();
        let wait_nofity = NOTIFY_ROPEN_STREAM.clone();
        
        loop {
            let mut map = wait_map.lock().await;
            let wait_stream = map.remove(session_key);
            drop(map);
            if wait_stream.is_some() {
                return Ok(wait_stream.unwrap().unwarp());
            }

            if let Err(_) = timeout(Duration::from_secs(5), wait_nofity.notified()).await {
                warn!("Timeout: ropen stream {} was not found within the time limit.",session_key);
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut,"Timeout"));
            }  
        }
    }
}

#[async_trait]
impl Tunnel for RTcpTunnel {
    async fn ping(&self) -> Result<(), std::io::Error> {
        //self.post_ping();
        Ok(())
    }

    async fn open_stream(&self, dest_port: u16) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        if self.can_direct {
            let target_ip = self.peer_addr.ip();
            let target_addr = SocketAddr::new(target_ip, dest_port);
            info!("RTcp tunnel open direct stream to {}#{}", target_addr,self.target.as_str());
            let stream = tokio::net::TcpStream::connect(target_addr).await?;
            Ok(Box::new(stream))
        } else {
            //send ropen to target
            //generate 32byte session_key
            let random_bytes: [u8; 16] = rand::thread_rng().gen();
            let session_key = hex::encode(random_bytes);
            self.post_ropen(dest_port,session_key.as_str()).await;
            //wait new stream with session_key fomr target
            let stream = self.wait_ropen_stream(&session_key.as_str()).await?;
            Ok(Box::new(stream))
        }
    }

    async fn create_datagram_client(&self, dest_port: u16) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        unimplemented!()
    }
}

pub struct RTcpStreamListener {
    bind_addr: Url,
    listener: Option<tokio::net::TcpListener>,
}

#[async_trait]
impl StreamListener for RTcpStreamListener {
    async fn accept(&self) -> Result<(Box<dyn AsyncStream>, TunnelEndpoint), std::io::Error> {
        let listener = self.listener.as_ref().unwrap();
        let (stream, addr) = listener.accept().await?;
        Ok((Box::new(stream), TunnelEndpoint {
            device_id: addr.ip().to_string(),
            port: addr.port(),
        }))
    }
}

pub struct RTcpTunnelBuilder {
    tunnel_port: u16,
    this_device: String,
}

enum WaitStream {
    Waiting,
    OK(TcpStream),
}

impl WaitStream {
    fn unwarp(self) -> TcpStream {
        match self {
            WaitStream::OK(stream) => stream,
            _ => panic!("unwarp error"),
        }
    }
}

lazy_static! {
    static ref NOTIFY_ROPEN_STREAM: Arc<Notify> = {
        Arc::new(Notify::new())
    };

    static ref WAIT_ROPEN_STREAM_MAP: Arc<Mutex<HashMap<String, WaitStream>>> = {
        Arc::new(Mutex::new(HashMap::new()))
    };

    static ref RTCP_TUNNEL_MAP: Arc<Mutex<HashMap<String, RTcpTunnel>>> = {
        Arc::new(Mutex::new(HashMap::new()))
    };
}

impl RTcpTunnelBuilder {
    pub fn new(this_device_id:String,port:u16) -> RTcpTunnelBuilder {
        let result = RTcpTunnelBuilder {
            tunnel_port: port,
            this_device: this_device_id,
        };
        return result;
    }

    pub async fn start(&mut self) -> Result<(), std::io::Error> {
        //create a tcp listener for tunnel
        let bind_addr = format!("0.0.0.0:{}",self.tunnel_port);
        let rtcp_listener = TcpListener::bind(bind_addr).await?;
        let this_device = self.this_device.clone();
        task::spawn(async move {
            loop {
                let (mut stream, addr) = rtcp_listener.accept().await.unwrap();
                let this_device2 = this_device.clone();
                let notify_clone = NOTIFY_ROPEN_STREAM.clone();
                task::spawn(async move {
                    let first_package = RTcpTunnelPackage::read_package(Pin::new(&mut stream),true).await;
                    if first_package.is_err() {
                        error!("read first package error:{}",first_package.err().unwrap());
                        return;
                    }

                    let package = first_package.unwrap();
                    match package {
                        RTcpTunnelPackage::HelloStream(session_key) => {
                            //find waiting ropen stream by session_key
                            //bind stream to session
                            let mut wait_streams = WAIT_ROPEN_STREAM_MAP.lock().await;
                            let mut wait_session = wait_streams.get_mut(session_key.as_str());
                            if wait_session.is_none() {
                                error!("no wait session for {}",session_key);
                                let _ = stream.shutdown();
                                return;
                            }
                            let mut wait_session = wait_session.unwrap();
                            *wait_session = WaitStream::OK(stream);
                            notify_clone.notify_waiters();
                            return;
                        },
                        RTcpTunnelPackage::Hello(hello_package) => {
                            //verify hello.to is self
                            if hello_package.body.to_id != this_device2 {
                                error!("hello.to_id {} is not this device",hello_package.body.to_id);
                                return;
                            }
                            //create tunnel to hello.from
                            let tunnel = RTcpTunnel::new(
                                Url::parse(hello_package.body.from_id.as_str()).unwrap(),
                                false,
                                stream,
                            );

                            let mut all_tunnel = RTCP_TUNNEL_MAP.lock().await;
                            let mut_old_tunnel = all_tunnel.get(hello_package.body.from_id.as_str());
                            if mut_old_tunnel.is_some() {
                                warn!("tunnel to {} already exist",hello_package.body.from_id);
                                mut_old_tunnel.unwrap().close();
                                return;
                            }

                            all_tunnel.insert(hello_package.body.from_id.clone(),tunnel.clone());
                            tunnel.run().await;
                        },
                        _ => {
                            error!("un support first package type");
                        }
                    }

                });
            }
        });

        Ok(())
    }



}

#[async_trait]
impl TunnelBuilder for RTcpTunnelBuilder {
    async fn create_tunnel(&self,target:&Url) -> TunnelResult<Box<dyn TunnelBox>> {
        // lookup existing tunnel and resue it
        let mut all_tunnel =  RTCP_TUNNEL_MAP.lock().await;
        let tunnel = all_tunnel.get(target.as_str());
        if tunnel.is_some() {
            return Ok(Box::new(tunnel.unwrap().clone()));
        }

        // resolve target device ip
        //      try to connect target device
        //      if success, create tunnel and insert to tunnel map
        let host = target.host_str().unwrap();
        let port = target.port().unwrap_or(2980);
        let remote_addr = format!("{}:{}",host,port);
        let tunnel_stream = tokio::net::TcpStream::connect(remote_addr.clone()).await;
        if tunnel_stream.is_err() {
            warn!("connect to {} error:{}",remote_addr,tunnel_stream.err().unwrap());
            return Err(TunnelError::ConnectError(format!("connect to {} error.",remote_addr)));
        }
        let tunnel_stream = tunnel_stream.unwrap();
        let tunnel = RTcpTunnel::new(target.clone(),true,tunnel_stream);
        all_tunnel.insert(target.to_string(),tunnel.clone());
        let result:TunnelResult<Box<dyn TunnelBox>> = Ok(Box::new(tunnel.clone()));
        task::spawn(async move {
            tunnel.run().await;
        });

        return result;

    }

    async fn create_listener(&self, bind_url: &Url) -> TunnelResult<Box<dyn StreamListener>> {
        unimplemented!("create_listener not implemented")
    }

    async fn create_datagram_server(&self, bind_url: &Url) -> TunnelResult<Box<dyn DatagramServerBox>> {
        unimplemented!("create_datagram_server not implemented")
    }
}


// how to test
// run cyfs-gateway1 @ vps with public ip
// run cyfs-gateway2 @ local with lan ip
// let cyfs-gateway2 connect to cyfs-gateway1
// config tcp://cyfs-gateway1:9000 to rtcp://cyfs-gateway2:8000
// then can acess http://cyfs-gateway1.w3.buckyos.io:9000 like access http://cyfs-gateway2:8000 