use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use hickory_client::client::{AsyncClient, ClientHandle};
use hickory_client::op::DnsResponse;
use hickory_client::proto::iocompat::AsyncIoTokioAsStd;
use hickory_client::rr::{DNSClass, Name, RData, RecordType};
use hickory_client::tcp::TcpClientStream;
use hickory_client::udp::UdpClientStream;
use tokio::net::UdpSocket;
use crate::{DnsTxtCodec, NameInfo, NSCmdRegister, NSError, NSErrorCode, NSProvider, NSResult};
use crate::error::{into_ns_err, ns_err};

pub struct DNSProvider {

}

impl DNSProvider {
    pub fn new() -> Self {
        Self {}
    }

    fn parse_dns_response(resp: DnsResponse) -> NSResult<NameInfo> {
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
        if txt_list.len() == 0 {
            return Err(ns_err!(NSErrorCode::NotFound, "txt data is empty"));
        }

        let txt = DnsTxtCodec::decode(txt_list)?;
        let txt = String::from_utf8_lossy(txt.as_slice()).to_string();
        return Ok(serde_json::from_str(txt.as_str()).map_err(into_ns_err!(NSErrorCode::InvalidData, "Failed to parse txt {}", txt))?);
    }
    async fn tcp_query(dns: &IpAddr, name: &Name) -> NSResult<NameInfo> {
        let (conn, sender) = TcpClientStream::<AsyncIoTokioAsStd<tokio::net::TcpStream>>::new(SocketAddr::new(dns.clone(), 53));
        let (mut dns_client, bg) = AsyncClient::new(conn, sender, None).await
            .map_err(into_ns_err!(NSErrorCode::DNSProtoError, "Failed to create async client {}", dns))?;

        tokio::spawn(bg);

        let resp = dns_client.query(name.clone(), DNSClass::IN, RecordType::TXT).await
            .map_err(into_ns_err!(NSErrorCode::DNSProtoError, "Failed to query dns"))?;
        Self::parse_dns_response(resp)
    }

    async fn udp_query(dns: &IpAddr, name: &Name) -> NSResult<NameInfo> {
        let conn = UdpClientStream::<UdpSocket>::new(SocketAddr::new(dns.clone(), 53));
        let (mut dns_client, bg) = AsyncClient::connect(conn).await
            .map_err(into_ns_err!(NSErrorCode::DNSProtoError, "Failed to create async client {}", dns))?;

        tokio::spawn(bg);

        let resp = dns_client.query(name.clone(), DNSClass::IN, RecordType::TXT).await
            .map_err(into_ns_err!(NSErrorCode::DNSProtoError, "Failed to query dns"))?;
        Self::parse_dns_response(resp)
    }
}
#[async_trait::async_trait]
impl NSProvider for DNSProvider {
    async fn load(&self, _cmd_register: &NSCmdRegister) -> NSResult<()> {
        Ok(())
    }

    async fn query(&self, name: &str) -> NSResult<NameInfo> {
        log::debug!("start dns query {}", name);
        let dns_list = sfo_net_utils::system_nameservers().map_err(into_ns_err!(NSErrorCode::InvalidData, "Failed to get system nameservers"))?;
        let name = Name::from_str(name).map_err(into_ns_err!(NSErrorCode::InvalidData, "Failed to parse name"))?;

        for dns in dns_list.iter().filter(|x| x.is_ipv4()) {
            log::debug!("query dns server {}", dns);
            match Self::tcp_query(dns, &name).await {
                Ok(info) => {
                    return Ok(info);
                }
                Err(e) => {
                    if e.code() == NSErrorCode::DNSProtoError {
                        match Self::udp_query(dns, &name).await {
                            Ok(info) => {
                                return Ok(info);
                            }
                            Err(e) => {
                                log::error!("query dns server {} err {}", dns, e);
                                continue;
                            }
                        }
                    } else {
                        log::error!("query dns server {} err {}", dns, e);
                        continue;
                    }
                }
            }
        }
        Err(ns_err!(NSErrorCode::InvalidData, "Failed to query dns"))
    }
}

