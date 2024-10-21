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
use std::collections::HashMap;
use std::net::{SocketAddr};
use std::sync::Arc;
use std::{fmt::Debug, net::IpAddr};
use std::str::FromStr;

use buckyos_kit::buckyos_get_unix_timestamp;
use tokio::net::tcp::ReadHalf;
use tokio::stream;
use tokio::sync::{Mutex, Notify};
use tokio::task;
use tokio::time::timeout;
use tokio::net::{TcpListener,TcpStream};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use log::*;
use rand::Rng;
use serde::{Deserialize, Serialize};
use async_trait::async_trait;
use url::form_urlencoded::ByteSerialize;
use url::Url;
use lazy_static::lazy_static;
use anyhow::Result;
use name_client::*;

use crate::{ tunnel, TunnelEndpoint, TunnelError, TunnelResult};
use crate::tunnel::{AsyncStream, DatagramClientBox, DatagramServerBox, StreamListener, Tunnel, TunnelBox, TunnelBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
enum RTcpTargetId {
    DeviceName(String),
    DeviceDid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RTcpTarget {
    id:RTcpTargetId,
    stack_port:u16,
    target_port:u16,
}

impl RTcpTarget {
    pub fn from_did(did:String) -> Self {
        RTcpTarget {
            id: RTcpTargetId::DeviceDid(did),
            stack_port: 2980,
            target_port: 0,
        }
    }

    pub fn from_name(name:String) -> Self {
        RTcpTarget {
            id: RTcpTargetId::DeviceName(name),
            stack_port: 2980,
            target_port: 0,
        }
    }

    pub fn get_id_str(&self) -> String {
        match self.id {
            RTcpTargetId::DeviceName(ref name) => name.clone(),
            RTcpTargetId::DeviceDid(ref did) => did.clone().replace(".", ":"),
        }
    }
}
fn parse_rtcp_url(url:&str) -> Option<RTcpTarget> {
    let url = Url::parse(url);
    if url.is_err() {
        return None;
    }
    let url = url.unwrap();

    let host = url.host();
    if host.is_none() {
        return None;
    }
    let host = host.unwrap();
    let result_id:RTcpTargetId;
    if host.to_string().starts_with("did.dev.") {
        result_id = RTcpTargetId::DeviceDid(host.to_string().replace(".", ":"));
    } else {
        result_id = RTcpTargetId::DeviceName(host.to_string());
    }

    let path = url.path();
    let path_parts = path.split('/').collect::<Vec<&str>>();
    let mut real_target_port = 0;
    if path_parts.len() > 1 {
        let target_port = path_parts[1].parse::<u16>();
        if target_port.is_ok() {
            real_target_port = target_port.unwrap();
        } else {
            return None;
        }
    } 
    let target = RTcpTarget {
        id: result_id,
        stack_port: url.port().unwrap_or(2980),
        target_port: real_target_port,
    };
    return Some(target);
}

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
    where T: Serialize  + Debug
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
    streamid: String,
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
                streamid: session_key,
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
        info!("|==> read rtcp package, len:{}",len);
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
    }

    pub async fn send_package<S,T>(mut stream:Pin<&mut S>,pkg:RTcpTunnelPackageImpl<T>) -> Result<()> 
        where 
            T: Serialize + Debug,
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
        info!("send package {} len:{} buflen:{}",json_str.as_str(),total_len,write_buf.len());
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
    target: RTcpTarget,
    can_direct: bool,
    peer_addr : SocketAddr,
    this_device:String,
    
    write_stream:Arc<Mutex<OwnedWriteHalf>>,
    read_stream:Arc<Mutex<OwnedReadHalf>>,
}

impl RTcpTunnel {
    fn new(this_device:String,target: &RTcpTarget,  can_direct: bool,stream:TcpStream) -> Self {
        let peer_addr = stream.peer_addr().unwrap();    
        let (read_stream,write_stream) = stream.into_split();
        let mut this_target = target.clone();
        this_target.target_port = 0;
        RTcpTunnel {
            target:this_target,
            can_direct:false,//Considering the limit of port mapping, the default configuration is configured as "NoDirect" mode
            peer_addr: peer_addr,
            this_device:this_device,
            read_stream : Arc::new(Mutex::new(read_stream)),
            write_stream : Arc::new(Mutex::new(write_stream)),
        }
    }

    pub async fn close(&self) {
        //let mut read_stream = self.read_stream.lock().await;
        //let mut read_stream:OwnedReadHalf = (*read_stream);
        //read_stream.shutdown().await;
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
                //if !self.can_direct {
                //    warn!("tunnel can not direct, ignore ropen"); 
                //    return Ok(());
                //}

                //1.open stream2 to real target
                //TODO: will support connect to other ip
                let target_addr = format!("127.0.0.1:{}",ropen_package.body.dest_port);
                info!("rtcp tunnel ropen: open real target stream to {}",target_addr);
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
                let mut target_addr = self.peer_addr.clone();
                target_addr.set_port(self.target.stack_port);
                let mut stream2 = tokio::net::TcpStream::connect(target_addr).await;
                if stream2.is_err() {
                    error!("open stream to remote {} error:{}",target_addr,stream2.err().unwrap());
                    let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 2);
                    let mut write_stream = self.write_stream.lock().await;
                    let mut write_stream = Pin::new(&mut *write_stream);
                    let _ = RTcpTunnelPackage::send_package(write_stream,ropen_resp_package).await?;
                    return Ok(());
                }

                //3. send ropen_resp
                let mut write_stream = self.write_stream.lock().await;
                let mut write_stream = Pin::new(&mut *write_stream);
                let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 0);
                RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;

                let mut stream2 = stream2.unwrap();
                RTcpTunnelPackage::send_hello_stream(&mut stream2,ropen_package.body.streamid.as_str()).await?;
                //4. 绑定两个stream
                task::spawn(async move {
                    let _copy_result = tokio::io::copy_bidirectional(&mut stream2,&mut stream1).await;
                    if _copy_result.is_err() {
                        error!("copy stream2 to stream1 error:{}",_copy_result.err().unwrap());
                    }
                });

       
                return Ok(());
            },
            RTcpTunnelPackage::ROpenResp(ropen_resp_package) => {
                //check result
                return Ok(());
            },
            RTcpTunnelPackage::Pong(pong_package) => {
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
            //info!("rtcp tunnel try read package from {}",self.peer_addr.to_string());
            let package = RTcpTunnelPackage::read_package(read_stream,false).await;
            //info!("rtcp tunnel read package from {} ok",self.target.as_str());
            if package.is_err() {
                error!("read package error:{:?}",package.err().unwrap());
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
        //let wait_map = WAIT_ROPEN_STREAM_MAP.clone();
        let wait_nofity = NOTIFY_ROPEN_STREAM.clone();
        let real_key = format!("{}_{}",self.this_device.as_str(),session_key);
        loop {
            let mut map = WAIT_ROPEN_STREAM_MAP.lock().await;
            let wait_stream = map.remove(real_key.as_str());
            
            if wait_stream.is_some() {
                match wait_stream.unwrap() {
                    WaitStream::OK(stream) => {
                        return Ok(stream);
                    },
                    WaitStream::Waiting => {
                        //do nothing
                        map.insert(real_key.clone(),WaitStream::Waiting);
                    }
                }
            }
            drop(map);
            if let Err(_) = timeout(Duration::from_secs(5), wait_nofity.notified()).await {
                warn!("Timeout: ropen stream {} was not found within the time limit.",real_key.as_str());
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut,"Timeout"));
            }  
        }
    }
}

#[async_trait]
impl Tunnel for RTcpTunnel {
    async fn ping(&self) -> Result<(), std::io::Error> {
        let timestamp = buckyos_get_unix_timestamp();
        let ping_package = RTcpPingPackage::new(0,timestamp);
        let mut write_stream = self.write_stream.lock().await;
        let mut write_stream = Pin::new(&mut *write_stream);
        let _ = RTcpTunnelPackage::send_package(write_stream,ping_package).await;
        Ok(())
    }

    async fn open_stream(&self, dest_port: u16) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        if self.can_direct {
            let target_ip = self.peer_addr.ip();
            let target_addr = SocketAddr::new(target_ip, dest_port);
            info!("RTcp tunnel open direct stream to {}#{}", target_addr,self.target.get_id_str());
            let stream = tokio::net::TcpStream::connect(target_addr).await?;
            Ok(Box::new(stream))
        } else {
            //send ropen to target
            //generate 32byte session_key
            let random_bytes: [u8; 16] = rand::thread_rng().gen();
            let session_key = hex::encode(random_bytes);
            let real_key = format!("{}_{}",self.this_device.as_str(),session_key);
            WAIT_ROPEN_STREAM_MAP.lock().await.insert(real_key.clone(),WaitStream::Waiting);
            info!("insert session_key {} to wait ropen stream map",real_key.as_str());
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

#[derive(Clone)]
pub struct RTcpStack {
    tunnel_port: u16,
    this_device: String,//name or did
}

enum WaitStream {
    Waiting,
    OK(TcpStream),
}

impl WaitStream {
    fn unwarp(self) -> TcpStream {
        match self {
            WaitStream::OK(stream) => stream,
            _ => panic!("unwarp WaitStream error"),
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

impl RTcpStack {
    pub fn new(this_device_id:String,port:u16) -> RTcpStack {
        let result = RTcpStack {
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
                info!("rtcp stack accept new tcp stream from {}",addr.clone());
                let this_device2 = this_device.clone();
                let notify_clone = NOTIFY_ROPEN_STREAM.clone();
                task::spawn(async move {
                    let first_package = RTcpTunnelPackage::read_package(Pin::new(&mut stream),true).await;
                    if first_package.is_err() {
                        error!("read first package error:{}",first_package.err().unwrap());
                        return;
                    }
                    info!("rtcp stack {} read first package ok",this_device2.as_str());
                    let package = first_package.unwrap();
                    match package {
                        RTcpTunnelPackage::HelloStream(session_key) => {
                            //find waiting ropen stream by session_key
                            //bind stream to session
                            let mut wait_streams = WAIT_ROPEN_STREAM_MAP.lock().await;
                            let clone_map : Vec<String> = wait_streams.keys().cloned().collect();
                            let real_key = format!("{}_{}",this_device2.as_str(),session_key.as_str());
                            let mut wait_session = wait_streams.get_mut(real_key.as_str());
                            if wait_session.is_none() {
                                error!("no wait session for {},map is {:?}",real_key.as_str(),clone_map);
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
                            //info!("hello.to_id {} is self",hello_package.body.to_id);
                            if hello_package.body.to_id != this_device2 {
                                error!("hello.to_id {} is not this device",hello_package.body.to_id);
                                return;
                            }
                            let from_did = hello_package.body.from_id.replace(":", ".");
                            let target_url = format!("rtcp://{}:{}",from_did.as_str(),hello_package.body.my_port);
                            let target = parse_rtcp_url(target_url.as_str());
                            if target.is_none() {
                                warn!("invalid incoming rtcp url:{}",target_url);
                                return;
                            }

                            let target: RTcpTarget = target.unwrap();
                            let tunnel = RTcpTunnel::new(
                                this_device2.clone(),
                                &target,
                                false,
                                stream,
                            );

                            let tunnel_key = format!("{}_{}",this_device2.as_str(),hello_package.body.from_id.as_str());
                            {
                                //info!("accept tunnel from {} try get lock",hello_package.body.from_id.as_str());
                                let mut all_tunnel = RTCP_TUNNEL_MAP.lock().await;
                                //info!("accept tunnel from {} get lock ok",hello_package.body.from_id.as_str());
                                let mut_old_tunnel = all_tunnel.get(tunnel_key.as_str());
                                if mut_old_tunnel.is_some() {
                                    warn!("tunnel {} already exist",tunnel_key.as_str());
                                    mut_old_tunnel.unwrap().close();
                                }

                                info!("accept tunnel from {}",hello_package.body.from_id.as_str());
                                all_tunnel.insert(tunnel_key.clone(),tunnel.clone());
                            }
                            info!("tunnel {} accept OK,start runing",tunnel_key.as_str());
                            tunnel.run().await;
                            
                            info!("tunnel {} end",tunnel_key.as_str());
                            {
                                let mut all_tunnel = RTCP_TUNNEL_MAP.lock().await;
                                all_tunnel.remove(tunnel_key.as_str());
                            }
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
impl TunnelBuilder for RTcpStack {
    async fn create_tunnel(&self,target:&Url) -> TunnelResult<Box<dyn TunnelBox>> {
        // lookup existing tunnel and resue it
        let target = parse_rtcp_url(target.as_str());
        if target.is_none() {       
            return Err(TunnelError::ConnectError(format!("invalid target url:{:?}",target)));
        }
        let target: RTcpTarget = target.unwrap();
        let target_id_str = target.get_id_str();

        let tunnel_key = format!("{}_{}",self.this_device.as_str(),target_id_str.as_str());
        info!("create tunnel to {} ,tunnel key is {},try reuse",target_id_str.as_str(),tunnel_key.as_str());
        let mut all_tunnel =  RTCP_TUNNEL_MAP.lock().await;
        let tunnel = all_tunnel.get(tunnel_key.as_str());
        if tunnel.is_some() {
            info!("reuse tunnel {}",tunnel_key.as_str());
            return Ok(Box::new(tunnel.unwrap().clone()));
        }
       
        let device_ip = resolve_ip(target_id_str.as_str()).await;
        if device_ip.is_err() {
            warn!("cann't resolve target device {} ip.",target_id_str.as_str());
            return Err(TunnelError::ConnectError(format!("cann't resolve target device {} ip.",target_id_str.as_str())));
        }
        let device_ip = device_ip.unwrap();
        let port = target.stack_port;
        let remote_addr = format!("{}:{}",device_ip,port);
        info!("create tunnel to {} ,target addr is {}",target_id_str.as_str(),remote_addr.as_str());

        let tunnel_stream = tokio::net::TcpStream::connect(remote_addr.clone()).await;
        if tunnel_stream.is_err() {
            warn!("connect to {} error:{}",remote_addr,tunnel_stream.err().unwrap());
            return Err(TunnelError::ConnectError(format!("connect to {} error.",remote_addr)));
        }

        //send hello to target
        let mut tunnel_stream = tunnel_stream.unwrap();
        let hello_package = RTcpHelloPackage::new(
            0,
            self.this_device.clone(),
            target_id_str.clone(),
            self.tunnel_port,
            None,
        );
        let send_result = RTcpTunnelPackage::send_package(Pin::new(&mut tunnel_stream),hello_package).await;
        if send_result.is_err() {
            warn!("send hello package to {} error:{}",remote_addr,send_result.err().unwrap());
            return Err(TunnelError::ConnectError(format!("send hello package to {} error.",remote_addr)));
        }

        //add tunnel to map
        let tunnel = RTcpTunnel::new(self.this_device.clone(),&target,true,tunnel_stream);
        all_tunnel.insert(tunnel_key.clone(),tunnel.clone());
        drop(all_tunnel);

        let result:TunnelResult<Box<dyn TunnelBox>> = Ok(Box::new(tunnel.clone()));
        task::spawn(async move {
            info!("rtcp tunnel {} established, tunnel running",tunnel_key.as_str());
            tunnel.run().await;
            //remove tunnel from map
            let mut all_tunnel = RTCP_TUNNEL_MAP.lock().await;
            all_tunnel.remove(&tunnel_key);
            info!("rtcp tunnel {} end",tunnel_key.as_str());
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

#[cfg(test)]
mod tests {
    use url::Url;
    use super::*;
    use name_client::*;
    use name_lib::*;
    use buckyos_kit::*;
    #[tokio::test]
    async fn test_rtcp_url() {
        init_logging("test_rtcp_tunnel");
        
        let url1 = "rtcp://dev02";
        let target1 = parse_rtcp_url(url1);
        info!("target1: {:?}",target1);
        let url2 = "rtcp://dev02.devices.web3.buckyos.io:9000/3080";
        let target2 = parse_rtcp_url(url2);
        info!("target2: {:?}",target2);
        let url3 = "rtcp://did.dev.LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0:9000/3080";
        let target3 = parse_rtcp_url(url3);
        info!("target3: {:?}",target3);
        let url4 = "rtcp://did.dev.LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0:9000/3080/323";
        let target4 = parse_rtcp_url(url4);
        info!("target4: {:?}",target4);
        let url5 = "rtcp://dev02.devices.web3.buckyos.io:9000/snkpi/323";
        let target5 = parse_rtcp_url(url5);
        info!("target5: {:?}",target5);
    }

    #[tokio::test]
    async fn test_rtcp_tunnel() {
        init_logging("test_rtcp_tunnel");
        init_default_name_client().await.unwrap();
        add_nameinfo_cache("dev02", NameInfo::from_address("dev02", IpAddr::V4("127.0.0.1".parse().unwrap()))).await.unwrap();
        let mut tunnel_builder1 = RTcpStack::new("dev01".to_string(), 8000);
        tunnel_builder1.start().await.unwrap();

        let mut tunnel_builder2 = RTcpStack::new("dev02".to_string(), 9000);
        tunnel_builder2.start().await.unwrap();

        let tunnel_url = Url::parse("rtcp://dev02:9000").unwrap();
        let tunnel = tunnel_builder1.create_tunnel(&tunnel_url).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let stream = tunnel.open_stream(8888).await.unwrap();
        info!("stream1 ok ");
        tokio::time::sleep(Duration::from_secs(5)).await;

        let tunnel_url = Url::parse("rtcp://dev01:9000").unwrap();
        let tunnel2 = tunnel_builder2.create_tunnel(&tunnel_url).await.unwrap();
        let stream2 = tunnel2.open_stream(7890).await.unwrap();
        info!("stream2 ok ");
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
}
