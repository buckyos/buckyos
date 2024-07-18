use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use bucky_raw_codec::{RawConvertTo, RawFixedBytes, RawFrom};
use pnet::packet::ipv4::Ipv4Packet;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::{select, spawn};
use tokio::time::sleep;
use crate::cmds::{Cmd, CmdCode, CmdHeader, HelloReq, HelloResp};
use crate::error::{into_tun_err, tun_err, TunError, TunErrorCode, TunResult};
use futures::{SinkExt, StreamExt};
use futures::stream::{FuturesUnordered, SplitSink, SplitStream};
use tun::{TunPacket};
use tun::AsyncDevice;
use tun::TunPacketCodec;
use tokio_util::codec::{Decoder, Framed};

pub struct VpnClient {
    server_addr: String,
    client_key: String,
    online: Mutex<bool>,
}


impl VpnClient {
    pub fn new(server_addr: &str, client_key: &str) -> Self {
        Self {
            server_addr: server_addr.to_string(),
            client_key: client_key.to_string(),
            online: Mutex::new(false),
        }
    }

    async fn recv_cmd(recv: &mut ReadHalf<TcpStream>) -> TunResult<(CmdCode, Vec<u8>)> {
        let mut header_buf = [0u8; 16];
        let header_len = CmdHeader::raw_bytes().unwrap();
        recv.read_exact(&mut header_buf[0..header_len]).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
        let header = CmdHeader::clone_from_slice(header_buf.as_slice()).map_err(into_tun_err!(TunErrorCode::RawCodecError))?;
        let cmd_code = match header.cmd_code() {
            Ok(cmd_code) => cmd_code,
            Err(err) => {
                return Err(TunError::from((TunErrorCode::Failed, hex::encode(&header_buf[0..header_len]), err)));
            }
        };
        let mut cmd_body = vec![0u8; header.pkg_len() as usize];
        recv.read_exact(cmd_body.as_mut_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;

        Ok((cmd_code, cmd_body))
    }

    pub async fn connect(&self) -> TunResult<(String, ReadHalf<TcpStream>, WriteHalf<TcpStream>)> {
        log::info!("connect {}", self.server_addr.as_str());
        let stream = TcpStream::connect(self.server_addr.as_str()).await.map_err(into_tun_err!(TunErrorCode::ConnectFailed))?;
        let (mut recv, mut send) = tokio::io::split(stream);

        let hello_req = HelloReq {
            client_key: self.client_key.to_string(),
        };
        send.write_all(Cmd::new(CmdCode::HelloReq, hello_req).to_vec().unwrap().as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
        send.flush().await.map_err(into_tun_err!(TunErrorCode::IoError))?;

        let (cmd_code, cmd_body) = Self::recv_cmd(&mut recv).await?;
        if cmd_code != CmdCode::HelloResp {
            return Err(tun_err!(TunErrorCode::Failed, "cmd error {:?}", cmd_code));
        }
        let hello_resp = HelloResp::clone_from_slice(cmd_body.as_slice()).map_err(into_tun_err!(TunErrorCode::RawCodecError))?;
        if hello_resp.client_ip.is_none() {
            return Err(tun_err!(TunErrorCode::Failed, "unauth"));
        }

        log::info!("connect server success.ip {}", hello_resp.client_ip.as_ref().unwrap());
        Ok((hello_resp.client_ip.unwrap(), recv, send))
    }

    pub async fn start(self: &Arc<Self>) {
        let this = self.clone();
        spawn(async move {
            loop {
                if let Err(e) = this.client_proc().await {
                    log::error!("err {}", e);
                }
                sleep(Duration::from_secs(5)).await;
            }
        });
    }

    pub async fn wait_online(&self) -> TunResult<()> {
        loop {
            {
                let online = self.online.lock().unwrap();
                if *online {
                    return Ok(());
                }
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    pub async fn client_proc(&self) -> TunResult<()> {
        let (client_ip, mut recv, mut send) = self.connect().await?;

        let mut config = tun::Configuration::default();
        config.address(client_ip.as_str()).netmask((255, 255, 255, 0)).up();

        #[cfg(target_os = "linux")]
        config.platform(|config| {
            config.packet_information(true);
        });

        let dev = tun::create_as_async(&config).unwrap();
        let mut framed = dev.into_framed();
        let (mut framed_send, mut framed_recv) = framed.split();
        {
            let mut online = self.online.lock().unwrap();
            *online = true;
        }
        loop {
            if let Err(e) = Self::run_proc(&mut framed_recv, &mut framed_send, &mut recv, &mut send).await {
                if e.code() == TunErrorCode::TunError {
                    return Err(e);
                } else {
                    log::error!("run proc err {}", e);
                    loop {
                        sleep(Duration::from_secs(5)).await;
                        match self.connect().await {
                            Ok((_client_ip, new_recv, new_send)) => {
                                recv = new_recv;
                                send = new_send;
                                break;
                            },
                            Err(e) => {
                                log::error!("connect err {:?} msg {}", e.code(), e.msg());
                            }
                        }
                    }
                }
            }
        }
    }

    async fn run_proc(framed_recv: &mut SplitStream<Framed<AsyncDevice, TunPacketCodec>>,
                      framed_send: &mut SplitSink<Framed<AsyncDevice, TunPacketCodec>, TunPacket>,
                      recv: &mut ReadHalf<TcpStream>,
                      send: &mut WriteHalf<TcpStream>) -> TunResult<()> {
        let mut latest_active = std::time::Instant::now();
        let mut header_buf = vec![0u8; CmdHeader::raw_bytes().unwrap()];
        let mut body_buf = None;
        let mut buf_ref = header_buf.as_mut_slice();
        let mut is_header = true;
        let mut header = None;
        let mut heart_tick = tokio::time::interval(Duration::from_secs(10));
        let mut timeout_tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            select! {
                ret = recv.read(buf_ref) => {
                    let len = ret.map_err(into_tun_err!(TunErrorCode::IoError))?;
                    if len == 0 {
                        return Err(tun_err!(TunErrorCode::IoError, "eof"));
                    }
                    buf_ref = &mut buf_ref[len..];
                    while buf_ref.len() == 0 {
                        if is_header {
                            header = Some(CmdHeader::clone_from_slice(header_buf.as_slice()).map_err(into_tun_err!(TunErrorCode::RawCodecError))?);
                            is_header = false;
                            body_buf = Some(vec![0u8; header.as_ref().unwrap().pkg_len() as usize]);
                            buf_ref = body_buf.as_mut().unwrap().as_mut_slice();
                        } else {
                            buf_ref = header_buf.as_mut_slice();
                            is_header = true;
                            let header = header.as_ref().unwrap();
                            let cmd_body = body_buf.take().unwrap();
                            let cmd_code = header.cmd_code()?;
                            if cmd_code == CmdCode::Data {
                                let pkt = TunPacket::new(cmd_body);
                                if let Some(ip_pkg) = Ipv4Packet::new(pkt.get_bytes()) {
                                    log::debug!("dest {} src {}", ip_pkg.get_destination(), ip_pkg.get_source());
                                }
                                framed_send.send(pkt).await.map_err(into_tun_err!(TunErrorCode::TunError))?;
                            } else if cmd_code == CmdCode::HeartResp {
                                latest_active = std::time::Instant::now();
                            }
                        }
                    }
                },
                ret = framed_recv.next() => {
                    if ret.is_none() {
                        return Err(tun_err!(TunErrorCode::TunError, "tun exit"))
                    }
                    let pkt = ret.unwrap().map_err(into_tun_err!(TunErrorCode::TunError))?;

                    if let Some(ip_pkg) = Ipv4Packet::new(pkt.get_bytes()) {
                        log::debug!("dest {} src {}", ip_pkg.get_destination(), ip_pkg.get_source());
                    }
                    let data = pkt.get_bytes();
                    send.write_all(CmdHeader::new(CmdCode::Data, data.len() as u16).to_vec().unwrap().as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
                    send.write_all(data).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
                    send.flush().await.map_err(into_tun_err!(TunErrorCode::IoError))?;
                }
                _ = heart_tick.tick() => {
                    send.write_all(CmdHeader::new(CmdCode::Heart, 0).to_vec().unwrap().as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
                    send.flush().await.map_err(into_tun_err!(TunErrorCode::IoError))?;
                }
                _ = timeout_tick.tick() => {
                    if std::time::Instant::now().duration_since(latest_active) > Duration::from_secs(30) {
                        return Err(tun_err!(TunErrorCode::Timeout, "timeout"));
                    }
                }
            }
        }
    }

    // async fn channel(framed_recv: &'static mut SplitStream<Framed<AsyncDevice, TunPacketCodec>>,
    //                  framed_send: &'static mut SplitSink<Framed<AsyncDevice, TunPacketCodec>, TunPacket>,
    //                  recv: &'static mut ReadHalf<TcpStream>,
    //                  send: &'static mut WriteHalf<TcpStream>) -> TunResult<()> {
    //     let mut latest_active = Arc::new(Mutex::new(std::time::Instant::now()));
    //
    //     let latest = latest_active.clone();
    //     let net_recv = spawn(async move {
    //         let ret: TunResult<()> = async move {
    //             loop {
    //                 match Self::recv_cmd(recv).await {
    //                     Ok((cmd_code, cmd_body)) => {
    //                         if cmd_code == CmdCode::Data {
    //                             let pkt = TunPacket::new(cmd_body);
    //                             if let Some(ip_pkg) = Ipv4Packet::new(pkt.get_bytes()) {
    //                                 log::info!("dest {} src {}", ip_pkg.get_destination(), ip_pkg.get_source());
    //                             }
    //                             framed_send.send(pkt).await.map_err(into_tun_err!(TunErrorCode::TunError))?;
    //                         } else if cmd_code == CmdCode::HeartResp {
    //                             let mut latest = latest.lock().unwrap();
    //                             *latest = std::time::Instant::now();
    //                         }
    //                     }
    //                     Err(e) => {
    //                         return Err(e);
    //                     }
    //                 }
    //             }
    //         }.await;
    //         log::info!("net recv exit");
    //         ret
    //     });
    //     let tun_recv = spawn(async move {
    //         let ret: TunResult<()> = async move {
    //             loop {
    //                 select! {
    //                     ret = framed_recv.next() => {
    //                         if ret.is_none() {
    //                             return Err(tun_err!(TunErrorCode::TunError, "tun exit"))
    //                         }
    //                         let pkt = ret.unwrap().map_err(into_tun_err!(TunErrorCode::TunError))?;
    //
    //                         if let Some(ip_pkg) = Ipv4Packet::new(pkt.get_bytes()) {
    //                             log::info!("dest {} src {}", ip_pkg.get_destination(), ip_pkg.get_source());
    //                         }
    //                         let data = pkt.get_bytes();
    //                         send.write_all(CmdHeader::new(CmdCode::Data, data.len() as u16).to_vec().unwrap().as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
    //                         send.write_all(data).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
    //                         send.flush().await.map_err(into_tun_err!(TunErrorCode::IoError))?;
    //                     },
    //                     _ = sleep(Duration::from_secs(10)) => {
    //                         send.write_all(CmdHeader::new(CmdCode::Heart, 0).to_vec().unwrap().as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
    //                         send.flush().await.map_err(into_tun_err!(TunErrorCode::IoError))?;
    //                     }
    //                 }
    //             }
    //         }.await;
    //         ret
    //     });
    //     let latest = latest_active.clone();
    //     let heart_resp = spawn(async move {
    //         let ret: TunResult<()> = async move {
    //             loop {
    //                 sleep(Duration::from_secs(1)).await;
    //                 if std::time::Instant::now().duration_since(*latest.lock().unwrap()) > Duration::from_secs(30) {
    //                     return Err(tun_err!(TunErrorCode::Timeout, "timeout"));
    //                 }
    //             }
    //         }.await;
    //         ret
    //     });
    //
    //     let net_recv_handle = net_recv.abort_handle();
    //     let tun_recv_handle = tun_recv.abort_handle();
    //     let heart_resp_handle = heart_resp.abort_handle();
    //     select! {
    //         ret = net_recv => {
    //             tun_recv_handle.abort();
    //             heart_resp_handle.abort();
    //             match ret {
    //                 Ok(ret) => {
    //                     ret
    //                 },
    //                 Err(e) => {
    //                     Err(TunError::from((TunErrorCode::Failed, "net_recv", e)))
    //                 }
    //             }
    //         }
    //         ret = tun_recv => {
    //             net_recv_handle.abort();
    //             heart_resp_handle.abort();
    //             match ret {
    //                 Ok(ret) => {
    //                     ret
    //                 },
    //                 Err(e) => {
    //                     Err(TunError::from((TunErrorCode::Failed, "net_recv", e)))
    //                 }
    //             }
    //         }
    //         ret = heart_resp => {
    //             net_recv_handle.abort();
    //             tun_recv_handle.abort();
    //             match ret {
    //                 Ok(ret) => {
    //                     ret
    //                 },
    //                 Err(e) => {
    //                     Err(TunError::from((TunErrorCode::Failed, "net_recv", e)))
    //                 }
    //             }
    //         }
    //     }
    //
    // }
}
