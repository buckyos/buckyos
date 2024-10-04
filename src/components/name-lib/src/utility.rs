use std::net::{IpAddr, Ipv6Addr};
use std::str::FromStr;
use tokio::net::UdpSocket;
use std::net::ToSocketAddrs;
use serde::{Serialize,Deserialize};
use serde_json::json;
use thiserror::Error;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use crate::CURRENT_ZONE_CONFIG;
use crate::config::DeviceConfig;
use sysinfo::{Components, Disks, Networks, System};

#[derive(Error, Debug)]
pub enum NSError {
    #[error("Failed: {0}")]
    Failed(String),
    #[error("Invalid response")]
    InvalidData,
    #[error("{0} not found")]
    NotFound(String),
    #[error("decode txt record error")]
    DnsTxtEncodeError,
    #[error("forbidden")]
    Forbid,
    #[error("DNS protocl error: {0}")]
    DNSProtoError(String),
    #[error("Failed to serialize extra: {0}")]
    ReadLocalFileError(String),
    #[error("Failed to decode jwt {0}")]
    DecodeJWTError(String),
}

pub type NSResult<T> = Result<T, NSError>;

pub fn is_did(identifier: &str) -> bool {
    if identifier.starts_with("did:") {
        let parts: Vec<&str> = identifier.split(':').collect();
        return parts.len() == 3 && !parts[1].is_empty() && !parts[2].is_empty();
    }
    false
}


pub fn decode_jwt_claim_without_verify(jwt: &str) -> NSResult<serde_json::Value> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(NSError::Failed("parts.len != 3".to_string())); // JWT 应该由三个部分组成
    }
    let claims_part = parts[1];
    let claims_bytes = URL_SAFE_NO_PAD.decode(claims_part).map_err(|_| NSError::Failed("base64 decode error".to_string()))?;
    let claims_str = String::from_utf8(claims_bytes).map_err(|_| NSError::Failed("String::from_utf8 error".to_string()))?;
    let claims: serde_json::Value = serde_json::from_str(claims_str.as_str()).map_err(|_| NSError::Failed("serde_json::from_str error".to_string()))?;

    Ok(claims)
}

pub fn decode_json_from_jwt_with_default_pk(jwt: &str,jwk:&jsonwebtoken::jwk::Jwk) -> NSResult<serde_json::Value> {

    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        NSError::DecodeJWTError("JWT decode header error".to_string())
    })?;

    let public_key = DecodingKey::from_jwk(jwk).unwrap();
    let validation = Validation::new(header.alg);

    let decoded_token = decode::<serde_json::Value>(jwt, &public_key, &validation).map_err(
        |error| NSError::DecodeJWTError(format!("JWT decode error:{}",error))
    )?;

    let decoded_json = decoded_token.claims.as_object()
        .ok_or(NSError::DecodeJWTError("Invalid token".to_string()))?;

    let result_value = serde_json::Value::Object(decoded_json.clone());

    Ok(result_value)
}

pub fn decode_json_from_jwt_with_pk(jwt: &str,pk:&jsonwebtoken::DecodingKey) -> NSResult<serde_json::Value> {

    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        NSError::DecodeJWTError("JWT decode header error".to_string())
    })?;

    let validation = Validation::new(header.alg);

    let decoded_token = decode::<serde_json::Value>(jwt,pk, &validation).map_err(
        |error| NSError::DecodeJWTError(format!("JWT decode error:{}",error))
    )?;

    let decoded_json = decoded_token.claims.as_object()
        .ok_or(NSError::DecodeJWTError("Invalid token".to_string()))?;

    let result_value = serde_json::Value::Object(decoded_json.clone());

    Ok(result_value)
}

pub fn is_unicast_link_local_stable(ipv6: &Ipv6Addr) -> bool {
    ipv6.segments()[0] == 0xfe80
}
// describe a device runtime info
#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct DeviceInfo {
    pub hostname:String,
    pub device_type:String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip:Option<IpAddr>,//main_ip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_net_interface:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_id:Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sys_hostname : Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_daemon_ver:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_os_info:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_info:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage:Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_mem:Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_usage:Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_space:Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_usage:Option<u64>,
}

impl DeviceInfo {
    pub fn from_device_doc(device_doc:&DeviceConfig) -> Self {
        let mut device_info = DeviceInfo::new(device_doc.name.as_str());
        device_info.device_type = device_doc.device_type.clone();
        device_info.ip = device_doc.ip.clone();
        device_info.net_id = device_doc.net_id.clone();

        return device_info;
    }

    pub fn new(ood_string:&str) -> Self {
        //device_string format: hostname@[ip]#[netid]
        let ip :Option<IpAddr>;
        let net_id :Option<String>;
        let parts: Vec<&str> = ood_string.split('@').collect();
        let hostname = parts[0];

        if parts.len() > 1 {
            let ip_str = parts[1];
            let ip_result = IpAddr::from_str(ip_str);
            if ip_result.is_ok() {
                ip = Some(ip_result.unwrap());
            } else {
                ip = None;
            }
        } else {
            ip = None;
        }

        let parts: Vec<&str> = ood_string.split('#').collect();
        if parts.len() == 2{
            net_id = Some(parts[1].to_string());
        } else {
            net_id = None;
        }   

        DeviceInfo {
            hostname:hostname.to_string(),
            device_type:"ood".to_string(),
            ip:ip,
            main_net_interface:None,
            net_id:net_id,
            node_daemon_ver:None,
            base_os_info:None,
            cpu_info:None,
            cpu_usage:None,
            total_mem:None,
            mem_usage:None,
            total_space:None,
            disk_usage:None,
            sys_hostname:None,
        }
    }

    pub async fn auto_fill_by_system_info(&mut self) -> NSResult<()> {
        let mut sys = System::new_all();
        sys.refresh_all();

        let test_socket = UdpSocket::bind("0.0.0.0:0").await;
        if test_socket.is_ok(){
            let test_socket = test_socket.unwrap();
            test_socket.connect("8.8.8.8:80").await;
            let local_addr = test_socket.local_addr().unwrap();
            self.ip = Some(local_addr.ip());
        }

        // Get OS information
        self.base_os_info = Some(format!("{} {} {}",System::name().unwrap_or_default(), System::os_version().unwrap_or_default(), System::kernel_version().unwrap_or_default()));

        // Get CPU information
        if let Some(cpu) = sys.cpus().first() {
            self.cpu_info = Some(format!("{} @ {} MHz", cpu.brand(), cpu.frequency()));
            self.cpu_usage = Some(cpu.cpu_usage() as f32);
        }

        // Get memory information
        self.total_mem = Some(sys.total_memory());
        self.mem_usage = Some((sys.used_memory() as f32 / sys.total_memory() as f32) * 100.0);
        // Get hostname if not already set
        self.sys_hostname = Some(System::host_name().unwrap_or_default());

        Ok(())
    }

    pub fn is_wan_device(&self) -> bool {
        if self.net_id.is_some() {
            let net_id = self.net_id.as_ref().unwrap();
            if net_id.starts_with("wan") {
                return true;
            }
        } 
        return false;
    }

    fn resolve_hostname(hostname: &str) -> Option<std::net::IpAddr> {
        let addrs = (hostname, 0).to_socket_addrs().ok()?;
        let mut ipv6_addr :Option<IpAddr> = None;
        for addr in addrs {
            if addr.is_ipv4() {
                let ip = addr.ip();
                match ip {
                    IpAddr::V4(ipv4) => {
                        return Some(IpAddr::V4(ipv4));
                    },
                    _ => {}
                }

            } else if addr.is_ipv6() {
                let ip = addr.ip();
                match ip {
                    IpAddr::V6(ipv6) => {
                        if !is_unicast_link_local_stable(&ipv6) {
                            ipv6_addr = Some(IpAddr::V6(ipv6));
                        }
                    }
                    _ => {}
                }
            }
        }

        if ipv6_addr.is_some() {
            return Some(ipv6_addr.unwrap());
        }

        return None;
    }

    fn resolve_lan_hostname(hostname: &str) -> Option<std::net::IpAddr> {
        let addrs = (hostname, 0).to_socket_addrs().ok()?;
        let mut private_ipv6_addr :Option<IpAddr> = None;
        for addr in addrs {
            if addr.is_ipv4() {
                let ip = addr.ip();
                match ip {
                    IpAddr::V4(ipv4) => {
                        if ipv4.is_private() {
                            return Some(IpAddr::V4(ipv4));
                        }
                    },
                    _ => {}
                }

            } else if addr.is_ipv6() {
                let ip = addr.ip();
                match ip {
                    IpAddr::V6(ipv6) => {
                        if !is_unicast_link_local_stable(&ipv6) {
                            private_ipv6_addr = Some(IpAddr::V6(ipv6));
                        }
                    }
                    _ => {}
                }
            }
        }

        if private_ipv6_addr.is_some() {
            return Some(private_ipv6_addr.unwrap());
        }

        return None;
    }

    //resolve device dynamic ip from device info
    pub async fn resolve_ip(&self) -> NSResult<IpAddr> {
        if self.ip.is_some() {
            return Ok(self.ip.unwrap().clone());
        }

        if !self.is_wan_device() {
            let hostname = self.hostname.clone();
            let addr = Self::resolve_lan_hostname(hostname.as_str());
            if addr.is_some() {
                return Ok(addr.unwrap());
            }

            let hostname = format!("{}.local",self.hostname);
            let addr = Self::resolve_lan_hostname(hostname.as_str());
            if addr.is_some() {
                return Ok(addr.unwrap());
            }
        }

        //try resolve by SN-HTTP-RPC
        //TODO:
        let zone_config = CURRENT_ZONE_CONFIG.get();
        if zone_config.is_some() {
            let zone_config = zone_config.unwrap();
            let sn_url = zone_config.get_sn_url();
            if sn_url.is_some() {
                let sn_url = sn_url.unwrap();
                let client = kRPC::kRPC::new(sn_url.as_str(), &None);
                let result = client.call("get_device_info",json!({"id":self.hostname})).await;
                if result.is_ok() {
                    let device_info = result.unwrap();
                    if device_info.is_object() {
                        let device_info = device_info.as_object().unwrap();
                        if device_info.contains_key("ip") {
                            let ip = device_info.get("ip").unwrap();
                            return Ok(IpAddr::from_str(ip.as_str().unwrap()).unwrap());
                        }
                    }
                }
            }

            //try resolve by SN-DNS
            if zone_config.name.is_some(){
                let hostname = format!("{}.d.{}",self.hostname,zone_config.name.as_ref().unwrap());
                let addr_result = Self::resolve_hostname(hostname.as_str());
                if addr_result.is_some() {
                    return Ok(addr_result.unwrap());
                }
            }
        }
  
        Err(NSError::Failed("cann't resolve ip for device".to_string()))
    }
}



