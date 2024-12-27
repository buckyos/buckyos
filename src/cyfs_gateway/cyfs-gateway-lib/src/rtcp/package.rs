use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use std::fmt::Debug;
use std::pin::Pin;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmdType {
    UnknowProtocol = 0,
    Hello = 1,
    HelloAck = 2,
    Ping = 3,
    Pong = 4,
    ROpen = 5,
    ROpenResp = 6,
    Open = 7,
    OpenResp = 8,
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
            7 => CmdType::Open,
            8 => CmdType::OpenResp,
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
pub(crate) struct RTcpTunnelPackageImpl<T>
where
    T: Serialize + Debug,
{
    pub len: u16,
    pub json_pos: u8,
    pub cmd: u8,
    pub seq: u32,
    pub body: T,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct TunnelTokenPayload {
    pub to: String,
    pub from: String,
    pub xpub: String,
    pub exp: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct RTcpHelloBody {
    pub from_id: String,
    pub to_id: String,
    pub my_port: u16,
    pub tunnel_token: Option<String>, //jwt token ,payload is TunnelTokenPayload
}

pub(crate) type RTcpHelloPackage = RTcpTunnelPackageImpl<RTcpHelloBody>;
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
pub(crate) struct RTcpHelloAckBody {
    test_result: bool,
}
pub(crate) type RTcpHelloAckPackage = RTcpTunnelPackageImpl<RTcpHelloAckBody>;

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
pub(crate) struct RTcpPingBody {
    timestamp: u64,
}
pub(crate) type RTcpPingPackage = RTcpTunnelPackageImpl<RTcpPingBody>;

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
pub(crate) struct RTcpPongBody {
    timestamp: u64,
}
pub(crate) type RTcpPongPackage = RTcpTunnelPackageImpl<RTcpPongBody>;

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
pub(crate) struct RTcpROpenBody {
    pub streamid: String,

    pub dest_port: u16,
    // Dest host in ip or domain format, if none, then use default local ip
    pub dest_host: Option<String>,
}

pub(crate) type RTcpROpenPackage = RTcpTunnelPackageImpl<RTcpROpenBody>;

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
pub(crate) struct RTcpROpenRespBody {
    result: u32,
}
pub(crate) type RTcpROpenRespPackage = RTcpTunnelPackageImpl<RTcpROpenRespBody>;

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

// Same as RTcpROpenBody
#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct RTcpOpenBody {
    pub streamid: String,

    pub dest_port: u16,
    // Dest host in ip or domain format, if none, then use default local ip
    pub dest_host: Option<String>,
}

pub(crate) type RTcpOpenPackage = RTcpTunnelPackageImpl<RTcpOpenBody>;

impl RTcpOpenPackage {
    pub fn new(seq: u32, session_key: String, dest_port: u16, dest_host: Option<String>) -> Self {
        Self {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Open.into(),
            seq: seq,
            body: RTcpOpenBody {
                streamid: session_key,
                dest_port,
                dest_host,
            },
        }
    }

    pub fn from_json(seq: u32, json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpOpenBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "parse package error",
            ));
        }

        let package = Self {
            len: 0,
            json_pos: 0,
            cmd: CmdType::Open.into(),
            seq: seq,
            body: body.unwrap(),
        };
        Ok(package)
    }
}


#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct RTcpOpenRespBody {
    result: u32,
}
pub(crate) type RTcpOpenRespPackage = RTcpTunnelPackageImpl<RTcpOpenRespBody>;

impl RTcpOpenRespPackage {
    pub fn new(seq: u32, result: u32) -> Self {
        Self {
            len: 0,
            json_pos: 0,
            cmd: CmdType::OpenResp.into(),
            seq: seq,
            body: RTcpOpenRespBody { result },
        }
    }

    pub fn from_json(seq: u32, json_value: serde_json::Value) -> Result<Self, std::io::Error> {
        let body = serde_json::from_value::<RTcpOpenRespBody>(json_value);
        if body.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "parse package error",
            ));
        }

        let package = Self {
            len: 0,
            json_pos: 0,
            cmd: CmdType::OpenResp.into(),
            seq: seq,
            body: body.unwrap(),
        };
        Ok(package)
    }
}


#[derive(Clone, Debug)]
pub(crate) enum RTcpTunnelPackage {
    HelloStream(String),
    Hello(RTcpHelloPackage),
    HelloAck(RTcpHelloAckPackage),
    Ping(RTcpPingPackage),
    Pong(RTcpPongPackage),
    ROpen(RTcpROpenPackage),
    ROpenResp(RTcpROpenRespPackage),
    Open(RTcpOpenPackage),
    OpenResp(RTcpOpenRespPackage),
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

            //start read json
            let _len = json_pos - 2;
            let read_buf = &buf[(_len as usize)..];
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
                CmdType::Open => {
                    let result_package = RTcpOpenPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::Open(result_package));
                }
                CmdType::OpenResp => {
                    let result_package = RTcpOpenRespPackage::from_json(seq, package_value)?;
                    return Ok(RTcpTunnelPackage::OpenResp(result_package));
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
        // First hello package len is 0
        let total_len = 0;
        let mut write_buf: Vec<u8> = Vec::new();
        let bytes = u16::to_be_bytes(total_len);
        write_buf.extend_from_slice(&bytes);
        write_buf.extend_from_slice(session_key.as_bytes());
        stream.write_all(&write_buf).await?;
        Ok(())
    }
}
