#![allow(unused)]

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
seession_key:option<string> （用对方公钥加密的key,并有自己的签名）
}
后续所有命令都用tunel key 对称加密
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
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{fmt::Debug, net::IpAddr};

use aes::cipher::{KeyIvInit, StreamCipher};
use base64::{
    engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD, Engine as _,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use ctr::Ctr128BE;
use ed25519_dalek::SigningKey;
use hex::ToHex;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use sha2::{Digest, Sha256};
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

use name_lib::DID;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::stream;
use tokio::sync::{Mutex, Notify};
use tokio::task;
use tokio::time::timeout;

use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use anyhow::Result;
use async_trait::async_trait;
use lazy_static::lazy_static;
use log::*;
use name_client::*;
use name_lib::*;
use rand::Rng;
use serde::{Deserialize, Serialize};
use url::form_urlencoded::ByteSerialize;
use url::Url;

use crate::aes_stream::{AesCtr, EncryptedStream};
use crate::tunnel::{
    DatagramClientBox, DatagramServerBox, StreamListener, Tunnel, TunnelBox, TunnelBuilder,
};
use crate::{tunnel, TunnelEndpoint, TunnelError, TunnelResult};
use buckyos_kit::AsyncStream;

pub const DEFAULT_RTCP_STACK_PORT: u16 = 2980;

#[derive(Debug, Clone, PartialEq)]
enum RTcpTargetId {
    DeviceName(String),
    DeviceDid(DID),
}

impl RTcpTargetId {
    pub fn from_str(value: &str) -> Option<Self> {
        if value.ends_with(".did") {
            let did = DID::from_host_name(value)?;
            return Some(RTcpTargetId::DeviceDid(did));
        }
        if value.starts_with("did:") {
            let did = DID::from_str(value)?;
            return Some(RTcpTargetId::DeviceDid(did));
        }
        Some(RTcpTargetId::DeviceName(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RTcpTarget {
    id: RTcpTargetId,
    stack_port: u16,
    target_port: u16,
}

impl RTcpTarget {
    pub fn new(hostname: &str, stack_port: u16, target_port: u16) -> Result<Self> {
        let id = RTcpTargetId::from_str(hostname);
        if id.is_none() {
            return Err(anyhow::anyhow!("invalid hostname:{}", hostname));
        }
        Ok(RTcpTarget {
            id: id.unwrap(),
            stack_port,
            target_port,
        })
    }

    pub fn from_did_str(did: String) -> Self {
        let did = DID::from_str(&did).unwrap();
        RTcpTarget {
            id: RTcpTargetId::DeviceDid(did),
            stack_port: DEFAULT_RTCP_STACK_PORT,
            target_port: 80,
        }
    }

    pub fn from_hostname(name: String) -> Self {
        RTcpTarget {
            id: RTcpTargetId::DeviceName(name),
            stack_port: DEFAULT_RTCP_STACK_PORT,
            target_port: 80,
        }
    }

    pub fn get_id_str(&self) -> String {
        match self.id {
            RTcpTargetId::DeviceName(ref name) => name.clone(),
            RTcpTargetId::DeviceDid(ref did) => did.to_host_name(),
        }
    }
}
fn parse_rtcp_url(url: &str) -> Option<RTcpTarget> {
    let url = Url::parse(url);
    if url.is_err() {
        return None;
    }
    let url = url.unwrap();
    if url.scheme() != "rtcp" {
        return None;
    }

    let mut stack_port = DEFAULT_RTCP_STACK_PORT;
    if url.username().len() > 0 {
        let _port = url.username().parse::<u16>();
        if _port.is_ok() {
            stack_port = _port.unwrap();
        }
    }

    let host = url.host();
    if host.is_none() {
        return None;
    }
    let host = host.unwrap();
    let result_id: RTcpTargetId;

    let target_did = DID::from_host_name(host.to_string().as_str());
    if target_did.is_some() {
        result_id = RTcpTargetId::DeviceDid(target_did.unwrap());
    } else {
        result_id = RTcpTargetId::DeviceName(host.to_string());
    }

    let target = RTcpTarget {
        id: result_id,
        stack_port: stack_port,
        target_port: url.port().unwrap_or(80),
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RTcpTunnelPackageImpl<T>
where
    T: Serialize + Debug,
{
    len: u16,
    json_pos: u8,
    cmd: u8,
    seq: u32,
    body: T,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TunnelTokenPayload {
    to: String,
    from: String,
    xpub: String,
    exp: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RTcpHelloBody {
    from_id: String,
    to_id: String,
    my_port: u16,
    tunnel_token: Option<String>, //jwt token ,payload is TunnelTokenPayload
}
type RTcpHelloPackage = RTcpTunnelPackageImpl<RTcpHelloBody>;
impl RTcpHelloPackage {
    pub fn new(
        seq: u32,
        from_id: String,
        to_id: String,
        my_port: u16,
        tunnel_token: Option<String>,
    ) -> Self {
        RTcpHelloPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Hello.into(),
            seq: seq,
            body: RTcpHelloBody {
                from_id,
                to_id,
                my_port,
                tunnel_token,
            },
        }
    }

    pub fn from_json(seq: u32, json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpHelloBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "parse package error",
            ));
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RTcpHelloAckBody {
    test_result: bool,
}
type RTcpHelloAckPackage = RTcpTunnelPackageImpl<RTcpHelloAckBody>;
impl RTcpHelloAckPackage {
    pub fn new(seq: u32, test_result: bool) -> Self {
        RTcpHelloAckPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::HelloAck.into(),
            seq: seq,
            body: RTcpHelloAckBody { test_result },
        }
    }

    pub fn from_json(seq: u32, json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpHelloAckBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "parse package error",
            ));
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RTcpPingBody {
    timestamp: u64,
}
type RTcpPingPackage = RTcpTunnelPackageImpl<RTcpPingBody>;
impl RTcpPingPackage {
    pub fn new(seq: u32, timestamp: u64) -> Self {
        RTcpPingPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Ping.into(),
            seq: seq,
            body: RTcpPingBody { timestamp },
        }
    }

    pub fn from_json(seq: u32, json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpPingBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "parse package error",
            ));
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RTcpPongBody {
    timestamp: u64,
}
type RTcpPongPackage = RTcpTunnelPackageImpl<RTcpPongBody>;
impl RTcpPongPackage {
    pub fn new(seq: u32, timestamp: u64) -> Self {
        RTcpPongPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Pong.into(),
            seq: seq,
            body: RTcpPongBody { timestamp },
        }
    }

    pub fn from_json(seq: u32, json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpPongBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "parse package error",
            ));
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RTcpROpenBody {
    streamid: String,

    dest_port: u16,
    // Dest host in ip or domain format, if none, then use default local ip
    dest_host: Option<String>,
}

type RTcpROpenPackage = RTcpTunnelPackageImpl<RTcpROpenBody>;

impl RTcpROpenPackage {
    pub fn new(seq: u32, session_key: String, dest_port: u16, dest_host: Option<String>) -> Self {
        RTcpROpenPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::ROpen.into(),
            seq: seq,
            body: RTcpROpenBody {
                streamid: session_key,
                dest_port,
                dest_host,
            },
        }
    }

    pub fn from_json(seq: u32, json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpROpenBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "parse package error",
            ));
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RTcpROpenRespBody {
    result: u32,
}
type RTcpROpenRespPackage = RTcpTunnelPackageImpl<RTcpROpenRespBody>;
impl RTcpROpenRespPackage {
    pub fn new(seq: u32, result: u32) -> Self {
        RTcpROpenRespPackage {
            len: 0,
            json_pos: 0,
            cmd: CmdType::ROpenResp.into(),
            seq: seq,
            body: RTcpROpenRespBody { result },
        }
    }

    pub fn from_json(seq: u32, json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpROpenRespBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "parse package error",
            ));
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

const TUNNEL_KEY_DEFAULT: [u8; 32] = [6; 32];

impl RTcpTunnelPackage {
    pub async fn read_package<'a, S>(
        mut stream: Pin<&'a mut S>,
        is_first_package: bool,
        source_info: &str,
    ) -> Result<RTcpTunnelPackage, std::io::Error>
    where
        S: AsyncReadExt + 'a,
    {
        let mut buf = [0; 2];
        //info!("try read 2 bytespackage len");
        stream.read_exact(&mut buf).await?;
        let len = u16::from_be_bytes(buf);
        info!("{}==> rtcp package, len:{}", source_info, len);
        if len == 0 {
            if !is_first_package {
                error!("HelloStream MUST be first package.");
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "first package is not HelloStream",
                ));
            }

            let mut buf = [0; 32];

            stream.read_exact(&mut buf).await?;
            let session_key = String::from_utf8_lossy(&buf);
            return Ok(RTcpTunnelPackage::HelloStream(session_key.to_string()));
        } else {
            let _len = len - 2;
            let mut buf = vec![0; _len as usize];
            //info!("read package data len:{}",_len);
            stream.read_exact(&mut buf).await?;

            let mut pos = 0;
            let json_pos = buf[pos];
            if json_pos < 6 {
                error!("json_pos is invalid");
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "json_pos is invalid",
                ));
            }
            pos = pos + 1;

            let read_buf = &buf[pos..pos + 1];
            let cmd_type = read_buf[0];
            pos = pos + 1;

            let read_buf = &buf[pos..pos + 4];
            let seq = u32::from_be_bytes(read_buf.try_into().unwrap());
            pos += 4;

            //start read json
            let _len = json_pos - 2;
            let mut read_buf = &buf[(_len as usize)..];
            //let base64_str: std::borrow::Cow<'_, str> = String::from_utf8_lossy(read_buf);
            let json_str = String::from_utf8_lossy(read_buf);

            //info!("read json:{}",json_str);
            let package_value = serde_json::from_str(json_str.as_ref());
            if package_value.is_err() {
                error!("parse package error:{}", package_value.err().unwrap());
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "parse package error",
                ));
            }
            let package_value = package_value.unwrap();

            let cmd = CmdType::from(cmd_type);
            match cmd {
                CmdType::Hello => {
                    let result_package = RTcpHelloPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::Hello(result_package));
                }
                CmdType::HelloAck => {
                    let result_package = RTcpHelloAckPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::HelloAck(result_package));
                }
                CmdType::Ping => {
                    let result_package: RTcpTunnelPackageImpl<RTcpPingBody> =
                        RTcpPingPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::Ping(result_package));
                }
                CmdType::Pong => {
                    let result_package = RTcpPongPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::Pong(result_package));
                }
                CmdType::ROpen => {
                    let result_package = RTcpROpenPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::ROpen(result_package));
                }
                CmdType::ROpenResp => {
                    let result_package = RTcpROpenRespPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::ROpenResp(result_package));
                }
                _ => {
                    error!("unsupport package type");
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "unsupport package type",
                    ));
                }
            }
        }
    }

    pub async fn send_package<S, T>(
        mut stream: Pin<&mut S>,
        pkg: RTcpTunnelPackageImpl<T>,
    ) -> Result<()>
    where
        T: Serialize + Debug,
        S: AsyncWriteExt,
    {
        //encode package to json
        let json_body = serde_json::to_string(&pkg.body).unwrap();
        let body_bytes = json_body.as_bytes();

        //let base64_str = URL_SAFE_NO_PAD.encode(json_str.as_bytes());
        let json_pos: u8 = 2 + 1 + 1 + 4;
        let total_len = 2 + 1 + 1 + 4 + body_bytes.len();
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
        write_buf.extend_from_slice(body_bytes);

        info!(
            "send package {} len:{} buflen:{}",
            json_body.as_str(),
            total_len,
            write_buf.len()
        );
        stream.write_all(&write_buf).await?;

        Ok(())
    }

    pub async fn send_hello_stream(
        stream: &mut TcpStream,
        session_key: &str,
    ) -> Result<(), anyhow::Error> {
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
    fn new(
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
                let mut write_stream = Pin::new(&mut *write_stream);
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
                    .map_err(|op| anyhow::format_err!("decode streamid error"))?;

                let mut raw_stream_to_target =
                    tokio::net::TcpStream::connect(target_addr.clone()).await;
                if raw_stream_to_target.is_err() {
                    error!(
                        "open raw tcp stream to target {} error:{}",
                        target_addr,
                        raw_stream_to_target.err().unwrap()
                    );
                    let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 1);
                    let mut write_stream = self.write_stream.lock().await;
                    let mut write_stream = Pin::new(&mut *write_stream);
                    let _ =
                        RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;
                    return Ok(());
                }
                let mut raw_stream_to_target = raw_stream_to_target.unwrap();
                //2.open stream to remote and send hello stream
                let mut target_addr = self.peer_addr.clone();
                target_addr.set_port(self.target.stack_port);
                let mut rtcp_stream = tokio::net::TcpStream::connect(target_addr).await;
                if rtcp_stream.is_err() {
                    error!(
                        "open rtcp stream to remote {} error:{}",
                        target_addr,
                        rtcp_stream.err().unwrap()
                    );
                    let ropen_resp_package = RTcpROpenRespPackage::new(ropen_package.seq, 2);
                    let mut write_stream = self.write_stream.lock().await;
                    let mut write_stream = Pin::new(&mut *write_stream);
                    let _ =
                        RTcpTunnelPackage::send_package(write_stream, ropen_resp_package).await?;
                    return Ok(());
                }

                //3. send ropen_resp
                let mut write_stream = self.write_stream.lock().await;
                let mut write_stream = Pin::new(&mut *write_stream);
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
            RTcpTunnelPackage::ROpenResp(ropen_resp_package) => {
                //check result
                return Ok(());
            }
            RTcpTunnelPackage::Pong(pong_package) => {
                return Ok(());
            }
            _ => {
                error!("unsupport package type");
                return Ok(());
            }
        }
        return Ok(());
    }

    async fn run(self) {
        let source_info = self.target.get_id_str();
        let mut read_stream = self.read_stream.lock().await;
        //let read_stream = self.read_stream.clone();
        loop {
            //等待超时 或 收到一个package
            //超时，基于last_active发送ping包,3倍超时时间后，关闭连接
            //收到一个package，处理package
            //   如果是req包，则处理逻辑后，发送resp包
            //   如果是resp包，则先找到对应的req包，然后处理逻辑

            let mut read_stream = Pin::new(&mut *read_stream);
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
        let mut write_stream = Pin::new(&mut *write_stream);
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
        let mut write_stream = Pin::new(&mut *write_stream);
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
        dest_port: u16,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
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
        Ok((
            Box::new(stream),
            TunnelEndpoint {
                device_id: addr.ip().to_string(),
                port: addr.port(),
            },
        ))
    }
}

#[derive(Clone)]
pub struct RTcpStack {
    tunnel_port: u16,
    this_device_hostname: String, //name or did
    this_device_ed25519_sk: Option<EncodingKey>,
    this_device_x25519_sk: Option<StaticSecret>,
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
    static ref NOTIFY_ROPEN_STREAM: Arc<Notify> = { Arc::new(Notify::new()) };
    static ref WAIT_ROPEN_STREAM_MAP: Arc<Mutex<HashMap<String, WaitStream>>> =
        { Arc::new(Mutex::new(HashMap::new())) };
    static ref RTCP_TUNNEL_MAP: Arc<Mutex<HashMap<String, RTcpTunnel>>> =
        { Arc::new(Mutex::new(HashMap::new())) };
}

impl RTcpStack {
    pub fn new(
        this_device_hostname: String,
        port: u16,
        private_key_pkcs8_bytes: Option<[u8; 48]>,
    ) -> RTcpStack {
        let mut this_device_x25519_sk = None;
        let mut this_device_ed25519_sk = None;
        if private_key_pkcs8_bytes.is_some() {
            let private_key_pkcs8_bytes = private_key_pkcs8_bytes.unwrap();
            //info!("rtcp stack ed25519 private_key pkcs8 bytes: {:?}",private_key_pkcs8_bytes);
            let encoding_key = EncodingKey::from_ed_der(&private_key_pkcs8_bytes);
            this_device_ed25519_sk = Some(encoding_key);

            let private_key_bytes = from_pkcs8(&private_key_pkcs8_bytes).unwrap();
            //info!("rtcp stack ed25519 private_key  bytes: {:?}",private_key_bytes);

            let x25519_private_key =
                ed25519_to_curve25519::ed25519_sk_to_curve25519(private_key_bytes);
            //info!("rtcp stack x25519 private_key_bytes: {:?}",x25519_private_key);
            this_device_x25519_sk = Some(x25519_dalek::StaticSecret::from(x25519_private_key));
        }

        let result = RTcpStack {
            tunnel_port: port,
            this_device_hostname,
            this_device_ed25519_sk: this_device_ed25519_sk, //for sign tunnel token
            this_device_x25519_sk: this_device_x25519_sk,   //for decode tunnel token from remote
        };
        return result;
    }

    //return (tunnel_token,aes_key,my_public_bytes)
    pub async fn generate_tunnel_token(
        &self,
        target_hostname: String,
    ) -> Result<(String, [u8; 32], [u8; 32]), TunnelError> {
        if self.this_device_ed25519_sk.is_none() {
            return Err(TunnelError::DocumentError(
                "this device ed25519 sk is none".to_string(),
            ));
        }

        let (auth_key, remote_did_id) = resolve_ed25519_auth_key(target_hostname.as_str())
            .await
            .map_err(|op| {
                TunnelError::DocumentError(format!(
                    "cann't resolve target device {} auth key:{}",
                    target_hostname.as_str(),
                    op
                ))
            })?;

        //info!("remote ed25519 auth_key: {:?}",auth_key);
        let remote_x25519_pk = ed25519_to_curve25519::ed25519_pk_to_curve25519(auth_key);
        //info!("remote x25519 pk: {:?}",remote_x25519_pk);

        let my_secret = EphemeralSecret::random();
        let my_public = PublicKey::from(&my_secret);
        let my_public_bytes = my_public.to_bytes();
        let my_public_hex = my_public.encode_hex();
        //info!("my_public_hex: {:?}",my_public_hex);
        let aes_key = RTcpStack::generate_aes256_key(my_secret, remote_x25519_pk);
        //info!("aes_key: {:?}",aes_key);
        //create jwt by tunnel token payload
        let tunnel_token_payload = TunnelTokenPayload {
            to: remote_did_id,
            from: self.this_device_hostname.clone(),
            xpub: my_public_hex,
            exp: buckyos_get_unix_timestamp() + 3600 * 2,
        };
        info!("send tunnel_token_payload: {:?}", tunnel_token_payload);
        let payload = serde_json::to_value(&tunnel_token_payload).map_err(|op| {
            TunnelError::ReasonError(format!("encode tunnel token payload error:{}", op))
        })?;

        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = None;
        header.typ = None;
        let tunnel_token = encode(
            &header,
            &payload,
            &self.this_device_ed25519_sk.as_ref().unwrap(),
        );
        if tunnel_token.is_err() {
            let err_str = tunnel_token.err().unwrap().to_string();
            return Err(TunnelError::ReasonError(err_str));
        }
        let tunnel_token = tunnel_token.unwrap();

        Ok((tunnel_token, aes_key, my_public_bytes))
    }

    pub fn generate_aes256_key(
        this_private_key: EphemeralSecret,
        x25519_public_key: [u8; 32],
    ) -> [u8; 32] {
        //info!("will create share sec with remote x25519 pk: {:?}",x25519_public_key);
        let x25519_public_key = x25519_dalek::PublicKey::from(x25519_public_key);
        let shared_secret = this_private_key.diffie_hellman(&x25519_public_key);

        let mut hasher = Sha256::new();
        hasher.update(shared_secret.as_bytes());
        let key_bytes = hasher.finalize();
        return key_bytes.try_into().unwrap();
        //return shared_secret.as_bytes().clone();
    }

    pub async fn decode_tunnel_token(
        this_private_key: &StaticSecret,
        token: String,
        from_hostname: String,
    ) -> Result<([u8; 32], [u8; 32]), TunnelError> {
        let (ed25519_pk, from_did) = resolve_ed25519_auth_key(from_hostname.as_str())
            .await
            .map_err(|op| {
                TunnelError::DocumentError(format!(
                    "cann't resolve from device {} auth key:{}",
                    from_hostname.as_str(),
                    op
                ))
            })?;

        let from_public_key = DecodingKey::from_ed_der(&ed25519_pk);

        let tunnel_token_payload = decode::<TunnelTokenPayload>(
            token.as_str(),
            &from_public_key,
            &Validation::new(Algorithm::EdDSA),
        );
        if tunnel_token_payload.is_err() {
            return Err(TunnelError::DocumentError(
                "decode tunnel token error".to_string(),
            ));
        }
        let tunnel_token_payload = tunnel_token_payload.unwrap();
        let tunnel_token_payload = tunnel_token_payload.claims;
        //info!("tunnel_token_payload: {:?}",tunnel_token_payload);
        let remomte_x25519_pk = hex::decode(tunnel_token_payload.xpub).unwrap();

        let remomte_x25519_pk: [u8; 32] = remomte_x25519_pk
            .try_into()
            .map_err(|op| TunnelError::ReasonError(format!("decode remote x25519 hex error")))?;
        //info!("remomte_x25519_pk: {:?}",remomte_x25519_pk);
        let aes_key = RTcpStack::get_aes256_key(this_private_key, remomte_x25519_pk.clone());
        //info!("aes_key: {:?}",aes_key);
        Ok((aes_key, remomte_x25519_pk))
    }

    pub fn get_aes256_key(
        this_private_key: &StaticSecret,
        remote_x25519_auth_key: [u8; 32],
    ) -> [u8; 32] {
        //info!("will get share sec with remote x25519 temp pk: {:?}",remote_x25519_auth_key);
        let x25519_public_key = x25519_dalek::PublicKey::from(remote_x25519_auth_key);
        let shared_secret = this_private_key.diffie_hellman(&x25519_public_key);

        let mut hasher = Sha256::new();
        hasher.update(shared_secret.as_bytes());
        let key_bytes = hasher.finalize();
        return key_bytes.try_into().unwrap();
        //return shared_secret.as_bytes().clone();
    }

    pub async fn start(&mut self) -> Result<(), std::io::Error> {
        //create a tcp listener for tunnel
        let bind_addr = format!("0.0.0.0:{}", self.tunnel_port);
        let rtcp_listener = TcpListener::bind(bind_addr).await?;
        let this_device = self.this_device_hostname.clone();
        //info!("rtcp stack this_device hostname: {}",this_device);
        let this_device_x25519_sk2 = self.this_device_x25519_sk.clone().unwrap();
        task::spawn(async move {
            loop {
                let (mut stream, addr) = rtcp_listener.accept().await.unwrap();
                info!("rtcp stack accept new tcp stream from {}", addr.clone());
                let this_device2 = this_device.clone();
                let notify_clone = NOTIFY_ROPEN_STREAM.clone();
                let this_device_x25519_sk = this_device_x25519_sk2.clone();
                task::spawn(async move {
                    let source_info = addr.to_string();
                    let first_package = RTcpTunnelPackage::read_package(
                        Pin::new(&mut stream),
                        true,
                        source_info.as_str(),
                    )
                    .await;
                    if first_package.is_err() {
                        error!("read first package error:{}", first_package.err().unwrap());
                        return;
                    }
                    info!("rtcp stack {} read first package ok", this_device2.as_str());
                    let package = first_package.unwrap();
                    match package {
                        RTcpTunnelPackage::HelloStream(session_key) => {
                            //find waiting ropen stream by session_key
                            let mut wait_streams = WAIT_ROPEN_STREAM_MAP.lock().await;
                            let clone_map: Vec<String> = wait_streams.keys().cloned().collect();
                            let real_key =
                                format!("{}_{}", this_device2.as_str(), session_key.as_str());
                            let mut wait_session = wait_streams.get_mut(real_key.as_str());
                            if wait_session.is_none() {
                                error!(
                                    "no wait session for {},map is {:?}",
                                    real_key.as_str(),
                                    clone_map
                                );
                                let _ = stream.shutdown();
                                return;
                            }
                            //bind stream to session
                            let mut wait_session = wait_session.unwrap();
                            *wait_session = WaitStream::OK(stream);
                            notify_clone.notify_waiters();
                            return;
                        }
                        RTcpTunnelPackage::Hello(hello_package) => {
                            //decode hello.body.tunnel_token
                            if hello_package.body.tunnel_token.is_none() {
                                error!("hello.body.tunnel_token is none");
                                return;
                            }
                            let token = hello_package.body.tunnel_token.as_ref().unwrap().clone();
                            let aes_key = RTcpStack::decode_tunnel_token(
                                &this_device_x25519_sk,
                                token,
                                hello_package.body.from_id.clone(),
                            )
                            .await;
                            if aes_key.is_err() {
                                error!("decode tunnel token error:{}", aes_key.err().unwrap());
                                return;
                            }
                            let (aes_key, random_pk) = aes_key.unwrap();
                            let target = RTcpTarget::new(
                                hello_package.body.from_id.as_str(),
                                hello_package.body.my_port,
                                80,
                            );
                            if target.is_err() {
                                error!("parser remote did error:{}", target.err().unwrap());
                                return;
                            }
                            let target = target.unwrap();
                            let tunnel = RTcpTunnel::new(
                                this_device2.clone(),
                                &target,
                                false,
                                stream,
                                aes_key,
                                random_pk,
                            );

                            let tunnel_key = format!(
                                "{}_{}",
                                this_device2.as_str(),
                                hello_package.body.from_id.as_str()
                            );
                            {
                                //info!("accept tunnel from {} try get lock",hello_package.body.from_id.as_str());
                                let mut all_tunnel = RTCP_TUNNEL_MAP.lock().await;
                                //info!("accept tunnel from {} get lock ok",hello_package.body.from_id.as_str());
                                let mut_old_tunnel = all_tunnel.get(tunnel_key.as_str());
                                if mut_old_tunnel.is_some() {
                                    warn!("tunnel {} already exist", tunnel_key.as_str());
                                    mut_old_tunnel.unwrap().close();
                                }

                                info!("accept tunnel from {}", hello_package.body.from_id.as_str());
                                all_tunnel.insert(tunnel_key.clone(), tunnel.clone());
                            }
                            info!("tunnel {} accept OK,start runing", tunnel_key.as_str());
                            tunnel.run().await;

                            info!("tunnel {} end", tunnel_key.as_str());
                            {
                                let mut all_tunnel = RTCP_TUNNEL_MAP.lock().await;
                                all_tunnel.remove(tunnel_key.as_str());
                            }
                        }
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
    async fn create_tunnel(&self, target: &Url) -> TunnelResult<Box<dyn TunnelBox>> {
        // lookup existing tunnel and resue it
        let target = parse_rtcp_url(target.as_str());
        if target.is_none() {
            return Err(TunnelError::ConnectError(format!(
                "invalid target url:{:?}",
                target
            )));
        }
        let target: RTcpTarget = target.unwrap();
        let target_id_str = target.get_id_str();

        let tunnel_key = format!(
            "{}_{}",
            self.this_device_hostname.as_str(),
            target_id_str.as_str()
        );
        //info!("will create tunnel to {} ,tunnel key is {},try reuse",target_id_str.as_str(),tunnel_key.as_str());
        let mut all_tunnel = RTCP_TUNNEL_MAP.lock().await;
        let tunnel = all_tunnel.get(tunnel_key.as_str());
        if tunnel.is_some() {
            info!("reuse tunnel {}", tunnel_key.as_str());
            return Ok(Box::new(tunnel.unwrap().clone()));
        }

        //1） resolve target auth-key and ip (rtcp base on tcp,so need ip)

        let device_ip = resolve_ip(target_id_str.as_str()).await;
        if device_ip.is_err() {
            warn!(
                "cann't resolve target device {} ip.",
                target_id_str.as_str()
            );
            return Err(TunnelError::ConnectError(format!(
                "cann't resolve target device {} ip.",
                target_id_str.as_str()
            )));
        }
        let device_ip = device_ip.unwrap();
        let port = target.stack_port;
        let remote_addr = format!("{}:{}", device_ip, port);
        //info!("create tunnel to {} ,target addr is {}",target_id_str.as_str(),remote_addr.as_str());

        //connect to target
        let tunnel_stream = tokio::net::TcpStream::connect(remote_addr.clone()).await;
        if tunnel_stream.is_err() {
            warn!(
                "connect to {} error:{}",
                remote_addr,
                tunnel_stream.err().unwrap()
            );
            return Err(TunnelError::ConnectError(format!(
                "connect to {} error.",
                remote_addr
            )));
        }
        //create tunnel token
        let (tunnel_token, aes_key, random_pk) =
            self.generate_tunnel_token(target_id_str.clone()).await?;

        //send hello to target
        let mut tunnel_stream = tunnel_stream.unwrap();
        let hello_package = RTcpHelloPackage::new(
            0,
            self.this_device_hostname.clone(),
            target_id_str.clone(),
            self.tunnel_port,
            Some(tunnel_token),
        );
        let send_result =
            RTcpTunnelPackage::send_package(Pin::new(&mut tunnel_stream), hello_package).await;
        if send_result.is_err() {
            warn!(
                "send hello package to {} error:{}",
                remote_addr,
                send_result.err().unwrap()
            );
            return Err(TunnelError::ConnectError(format!(
                "send hello package to {} error.",
                remote_addr
            )));
        }

        //create tunnel and add to map
        let tunnel = RTcpTunnel::new(
            self.this_device_hostname.clone(),
            &target,
            true,
            tunnel_stream,
            aes_key,
            random_pk,
        );
        all_tunnel.insert(tunnel_key.clone(), tunnel.clone());
        info!(
            "create tunnel {} ok,remote addr is {}",
            tunnel_key.as_str(),
            remote_addr.as_str()
        );
        drop(all_tunnel);

        let result: TunnelResult<Box<dyn TunnelBox>> = Ok(Box::new(tunnel.clone()));
        task::spawn(async move {
            info!(
                "rtcp tunnel {} established, tunnel running",
                tunnel_key.as_str()
            );
            tunnel.run().await;
            //remove tunnel from map
            let mut all_tunnel = RTCP_TUNNEL_MAP.lock().await;
            all_tunnel.remove(&tunnel_key);
            info!("rtcp tunnel {} end", tunnel_key.as_str());
        });

        return result;
    }

    async fn create_listener(&self, bind_url: &Url) -> TunnelResult<Box<dyn StreamListener>> {
        unimplemented!("create_listener not implemented")
    }

    async fn create_datagram_server(
        &self,
        bind_url: &Url,
    ) -> TunnelResult<Box<dyn DatagramServerBox>> {
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
    use super::*;
    use buckyos_kit::*;
    use name_client::*;
    use name_lib::*;
    use url::Url;
    #[tokio::test]
    async fn test_rtcp_url() {
        init_logging("test_rtcp_tunnel");

        let url1 = "rtcp://dev02";
        let target1 = parse_rtcp_url(url1);
        info!("target1: {:?}", target1);
        let url2 = "rtcp://dev02.devices.web3.buckyos.io:3080/";
        let target2 = parse_rtcp_url(url2);
        info!("target2: {:?}", target2);
        let url3 = "rtcp://9000@LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0.dev.did/";
        let target3 = parse_rtcp_url(url3);
        info!("target3: {:?}", target3);
        let url4 = "rtcp://8000@LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0.dev.did:3000/";
        let target4 = parse_rtcp_url(url4);
        info!("target4: {:?}", target4);
        let url5 = "rtcp://dev02.devices.web3.buckyos.io:9000/snkpi/323";
        let target5 = parse_rtcp_url(url5);
        info!("target5: {:?}", target5);
    }

    #[tokio::test]
    async fn test_rtcp_tunnel() {
        init_logging("test_rtcp_tunnel");
        init_default_name_client().await.unwrap();

        let (sk, sk_pkcs) = generate_ed25519_key();

        let pk = encode_ed25519_sk_to_pk_jwt(&sk);
        let pk_str = serde_json::to_string(&pk).unwrap();

        let mut name_info =
            NameInfo::from_address("dev02", IpAddr::V4("127.0.0.1".parse().unwrap()));
        name_info.did_document = Some(EncodedDocument::Jwt(pk_str.clone()));
        add_nameinfo_cache("dev02", name_info).await.unwrap();

        //add_did_cache("dev01", EncodedDocument::Jwt(pk_str.clone())).await.unwrap();
        //add_did_cache("dev02", EncodedDocument::Jwt(pk_str.clone())).await.unwrap();

        let mut tunnel_builder1 = RTcpStack::new("dev01".to_string(), 8000, Some(sk_pkcs.clone()));
        tunnel_builder1.start().await.unwrap();

        let mut tunnel_builder2 = RTcpStack::new("dev02".to_string(), 9000, Some(sk_pkcs.clone()));
        tunnel_builder2.start().await.unwrap();

        let tunnel_url = Url::parse("rtcp://9000@dev02/").unwrap();
        let tunnel = tunnel_builder1.create_tunnel(&tunnel_url).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let stream = tunnel.open_stream(8888).await.unwrap();
        info!("stream1 ok ");
        tokio::time::sleep(Duration::from_secs(5)).await;

        return;
        let tunnel_url = Url::parse("rtcp://8000@dev01").unwrap();
        let tunnel2 = tunnel_builder2.create_tunnel(&tunnel_url).await.unwrap();
        let stream2 = tunnel2.open_stream(7890).await.unwrap();
        info!("stream2 ok ");
        tokio::time::sleep(Duration::from_secs(20)).await;

        // test rudp with dev01 and dev02
        //let tunnel_url = Url::parse("rudp://dev02").unwrap();

        //let data_stream = tunnel.create_datagram_client(1000).await.unwrap();

        //let data_stream2 = tunnel2.create_datagram_server(1000).await.unwrap();
    }
}
