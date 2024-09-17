use std::net::SocketAddr;
use std::str::FromStr;
use hickory_client::client::{AsyncClient, ClientHandle};
use hickory_client::rr::{DNSClass, Name, RData, RecordType};
use hickory_client::udp::UdpClientStream;
use tokio::net::UdpSocket;
use crate::{DnsTxtCodec, NameInfo, NSCmdRegister, NSError, NSErrorCode, NSProvider, NSResult};

pub struct DNSProvider {

}

impl DNSProvider {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait::async_trait]
impl NSProvider for DNSProvider {
    async fn load(&self, _cmd_register: &NSCmdRegister) -> NSResult<()> {
        Ok(())
    }

    async fn query(&self, name: &str) -> NSResult<NameInfo> {
        let dns_list = sfo_net_utils::system_nameservers().map_err(|e| {
            NSError::new(NSErrorCode::InvalidData, format!("Failed to get system nameservers: {}", e))
        })?;
        let name = Name::from_str(name).map_err(|e| {
            NSError::new(NSErrorCode::InvalidData, format!("Failed to parse name: {}", e))
        })?;

        for dns in dns_list.iter().filter(|x| x.is_ipv4()) {
            let conn = UdpClientStream::<UdpSocket>::new(SocketAddr::new(dns.clone(), 53));
            let (mut dns_client, bg) = match AsyncClient::connect(conn).await {
                Ok(v) => v,
                Err(e) => {
                    log::info!("Failed to create async client: {}", e);
                    continue;
                }
            };

            // let (conn, sender) = TcpClientStream::<AsyncIoTokioAsStd<tokio::net::TcpStream>>::new(SocketAddr::new(dns.clone(), 53));
            // let (mut dns_client, bg) = AsyncClient::new(conn, sender, None).await.unwrap();
            tokio::spawn(bg);

            match dns_client.query(name.clone(), DNSClass::IN, RecordType::TXT).await {
                Ok(resp) => {
                    let mut txt_list = Vec::new();
                    for record in resp.answers() {
                        if record.record_type() == RecordType::TXT {
                            let data = record.data();
                            if data.is_some() {
                                let data = data.unwrap();
                                if let RData::TXT(txt) = data {
                                    for txt in txt.txt_data() {
                                        let txt = String::from_utf8_lossy(txt).to_string();
                                        txt_list.push(txt);
                                    }
                                }

                            }
                        }
                    }
                    let txt = DnsTxtCodec::decode(txt_list)?;
                    //let txt = String::from_utf8_lossy(txt.as_slice()).to_string();
                    return Ok(serde_json::from_str(txt.as_str()).map_err(|e| {
                        NSError::new(NSErrorCode::InvalidData, format!("Failed to parse txt {} err: {}", txt, e))
                    })?);
                }
                Err(e) => {
                    log::info!("Failed to query dns: {}", e);
                    continue;
                }
            }

        }
        Err(NSError::new(NSErrorCode::InvalidData, "Failed to query dns".to_string()))
    }
}

