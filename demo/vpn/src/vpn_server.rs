use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use bucky_raw_codec::{RawConvertTo, RawFixedBytes, RawFrom};
use pnet::packet::ipv4::Ipv4Packet;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::spawn;
use tokio::task::JoinHandle;
use crate::cmds::{Cmd, CmdCode, CmdHeader, HelloReq, HelloResp};
use crate::error::{into_tun_err, tun_err, TunErrorCode, TunResult};

struct Client {
    recv_handle: JoinHandle<()>,
    send: WriteHalf<TcpStream>,
}


pub struct VpnServer {
    listener: TcpListener,
    clients: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Client>>>>,
    client_config: HashMap<String, String>,
}

impl VpnServer {
    pub async fn bind<A: ToSocketAddrs>(addr: A, client_config: HashMap<String, String>) -> TunResult<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr).await.map_err(into_tun_err!(TunErrorCode::IoError))?,
            clients: Mutex::new(Default::default()),
            client_config,
        })
    }

    pub async fn start(self: &Arc<Self>) {
        let this = self.clone();
        spawn(async move {
            loop {
                match this.listener.accept().await {
                    Ok((socket, from_addr)) => {
                        if let Err(e) = this.accept(socket, from_addr).await {
                            log::error!("accept err {}", e);
                        }
                    }
                    Err(e) => {
                        log::error!("accept err {}", e)
                    }
                }
            }
        });
    }

    async fn recv_cmd(recv: &mut ReadHalf<TcpStream>) -> TunResult<(CmdCode, Vec<u8>)> {
        let mut header = [0u8; 16];
        recv.read_exact(&mut header[0..CmdHeader::raw_bytes().unwrap()]).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
        let header = CmdHeader::clone_from_slice(header.as_slice()).map_err(into_tun_err!(TunErrorCode::RawCodecError))?;
        let cmd_code = match header.cmd_code() {
            Ok(cmd_code) => cmd_code,
            Err(err) => {
                return Err(err);
            }
        };
        if header.pkg_len() == 0 {
            return Ok((cmd_code, vec![]));
        }
        let mut cmd_body = vec![0u8; header.pkg_len() as usize];
        recv.read_exact(cmd_body.as_mut_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;

        Ok((cmd_code, cmd_body))
    }

    async fn accept(self: &Arc<Self>, socket: TcpStream, from_addr: SocketAddr) -> TunResult<()> {
        let (mut recv, mut send) = tokio::io::split(socket);

        let (cmd_code, cmd_body) = Self::recv_cmd(&mut recv).await?;
        if cmd_code != CmdCode::HelloReq {
            return Err(tun_err!(TunErrorCode::Failed, "unexpect cmd {:?}", cmd_code));
        }

        let hello_req = HelloReq::clone_from_slice(cmd_body.as_slice()).map_err(into_tun_err!(TunErrorCode::RawCodecError))?;
        log::info!("recv client {} ip {}", hello_req.client_key, from_addr);
        let client_ip = self.client_config.get(&hello_req.client_key);
        let resp = HelloResp {
            client_ip: client_ip.map(|v| v.clone()),
        };
        send.write_all(Cmd::new(CmdCode::HelloResp, resp).to_vec().unwrap().as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
        send.flush().await.map_err(into_tun_err!(TunErrorCode::IoError))?;

        if client_ip.is_none() {
            return Ok(());
        }
        let ip = client_ip.clone().unwrap().clone();
        let this = self.clone();
        let handle: JoinHandle<()> = spawn(async move {
            loop {
                match this.client_recv_proc(&ip, &mut recv).await {
                    Ok(_) => {}
                    Err(e) => {
                        log::error!("{} recv err {}", ip, e);
                        let mut clients = this.clients.lock().unwrap();
                        clients.remove(&ip);
                        break;
                    }
                }
            }
        });

        let client = Client {
            recv_handle: handle,
            send,
        };

        let mut clients = self.clients.lock().unwrap();
        let ip = client_ip.clone().unwrap().clone();
        log::info!("new client {}", ip);
        clients.insert(ip, Arc::new(tokio::sync::Mutex::new(client)));

        Ok(())
    }

    async fn client_recv_proc(&self, ip: &String, recv: &mut ReadHalf<TcpStream>) -> TunResult<()> {
        let (cmd_code, cmd_body) = Self::recv_cmd(recv).await?;
        if cmd_code == CmdCode::Data {
            if let Some(ip_pkg) = Ipv4Packet::new(cmd_body.as_slice()) {
                let dest = ip_pkg.get_destination().to_string();

                let client = {
                    let clients = self.clients.lock().unwrap();
                    let client = clients.get(&dest);
                    if client.is_none() {
                        log::info!("can't find client {}", dest);
                        return Ok(());
                    }
                    client.unwrap().clone()
                };

                let mut client = client.lock().await;
                client.send.write_all(CmdHeader::new(CmdCode::Data, cmd_body.len() as u16).to_vec().unwrap().as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
                client.send.write_all(cmd_body.as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
                client.send.flush().await.map_err(into_tun_err!(TunErrorCode::IoError))?;
            }
        } else if cmd_code == CmdCode::Heart {
            let client = {
                let clients = self.clients.lock().unwrap();
                let client = clients.get(ip);
                if client.is_none() {
                    log::info!("can't find heart client {}", ip);
                    return Ok(());
                }
                client.unwrap().clone()
            };

            let mut client = client.lock().await;
            client.send.write_all(CmdHeader::new(CmdCode::HeartResp, 0).to_vec().unwrap().as_slice()).await.map_err(into_tun_err!(TunErrorCode::IoError))?;
            client.send.flush().await.map_err(into_tun_err!(TunErrorCode::IoError))?;
        }
        Ok(())
    }
}
