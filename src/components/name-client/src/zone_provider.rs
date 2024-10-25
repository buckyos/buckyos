// ZoneProvider 是 BuckyOS Zone Provider 目标是统一支持Zone内的名字解析和URL构造:
// 1. 核心功能 查询Zone内Device的实时信息）
// 2. 最小配置是ZoneConfig,但在连上SystemConfig后，可以做到更多事情
// 3. 未连接上SystemConfig时，基于ZoneConfig或ZoneSN进行查询。一旦连接上SystemConfig,则基于后者进行查询
// 4. 查询的输入: 
//        device_short_name （friendly name）
//        device_did
//        device_host_name.zone_fullname

#![allow(unused)]

use log::*;
use name_lib::ZoneConfig;
use std::net::{IpAddr, Ipv6Addr,SocketAddr,ToSocketAddrs};
use std::str::FromStr;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::{Value,json};
use sys_config::*;
use ::kRPC::*;

use crate::{DeviceInfo, EncodedDocument, NSError, NSProvider, NSResult, NameInfo, CURRENT_ZONE_CONFIG};


pub fn is_unicast_link_local_stable(ipv6: &Ipv6Addr) -> bool {
    ipv6.segments()[0] == 0xfe80
}

//ipv4 first host resolve
pub fn resolve_hostname(hostname: &str) -> Option<std::net::IpAddr> {
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


//ipv4 first local host resolve
pub fn resolve_lan_hostname(hostname: &str) -> Option<std::net::IpAddr> {
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
                },
                _ => {}
            }
        }
    }

    if private_ipv6_addr.is_some() {
        return Some(private_ipv6_addr.unwrap());
    }

    return None;
}

pub async fn resolve_ood_ip_by_info(ood_info: &DeviceInfo,zone_config:&ZoneConfig) -> NSResult<IpAddr> {
    if ood_info.ip.is_some() {
        return Ok(ood_info.ip.unwrap().clone());
    }

    let zone_short_name = zone_config.get_zone_short_name();
    

    if !ood_info.is_wan_device() {
        let hostname = format!("{}-{}",zone_short_name.as_str(),ood_info.hostname.as_str());
        let addr = resolve_lan_hostname(hostname.as_str());
        if addr.is_some() {
            return Ok(addr.unwrap());
        }

        let hostname = format!("{}-{}.local",zone_short_name.as_str(),ood_info.hostname.as_str());
        let addr = resolve_lan_hostname(hostname.as_str());
        if addr.is_some() {
            return Ok(addr.unwrap());
        }
    }

    //try resolve by HTTP-SN
    let sn_url = zone_config.get_sn_url();
    if sn_url.is_some() {
        let sn_url = sn_url.unwrap();
        info!("try resolve ood {} ip by sn: {}",ood_info.hostname.clone(),sn_url);
        let device_info = sn_get_device_info(sn_url.as_str(),None,
            zone_config.get_zone_short_name().as_str(),ood_info.hostname.as_str()).await;

        if device_info.is_ok() {
            let device_info = device_info.unwrap();
            return Ok(device_info.ip.unwrap());
        }
    }

    Err(NSError::Failed("cann't resolve ip for device".to_string()))
}


pub async fn get_system_config_service_url(this_device:Option<&DeviceInfo>,zone_config:&ZoneConfig,is_gateway:bool) -> NSResult<String> {
    if this_device.is_none() {
        return Ok(String::from("http://127.0.0.1:3200/kapi/system_config"));
    }

    let this_device = this_device.unwrap();
    let ood_string = zone_config.get_ood_string(this_device.hostname.as_str());
    if ood_string.is_some() {
        return Ok(String::from("http://127.0.0.1:3200/kapi/system_config"));
    }

    //this device is not ood, looking best ood for system config service
    let ood_info_str = zone_config.select_same_subnet_ood(this_device);
    if ood_info_str.is_some() {
        let ood_info = DeviceInfo::new(ood_info_str.unwrap().as_str(),None);
        info!("try connect to same subnet ood: {}",ood_info.hostname);
        let ood_ip = resolve_ood_ip_by_info(&ood_info,zone_config).await;
        if ood_ip.is_ok() {
            let ood_ip = ood_ip.unwrap();
            let server_url = format!("http://{}:3200/kapi/system_config",ood_ip);
            return Ok(server_url);
        }
    } 

    let ood_info_str = zone_config.select_wan_ood();
    if ood_info_str.is_some() {
        //try connect to wan ood
        let ood_info = DeviceInfo::new(ood_info_str.unwrap().as_str(),None);
        info!("try connect to wan ood: {}",ood_info.hostname);
        let ood_ip = resolve_ood_ip_by_info(&ood_info,zone_config).await;
        if ood_ip.is_ok() {
            let ood_ip = ood_ip.unwrap();
            let server_url = format!("http://{}:3200/kapi/system_config",ood_ip);
            return Ok(server_url);
        }
    }

    if !is_gateway {
        //connect to local cyfs_gateway,local cyfs-gateway will use tunnel connect to ood
        warn!("cann't connect to ood directly, try connect to system config service by local cyfs_gateway");
        return Ok(String::from("http://127.0.0.1:3180/kapi/system_config"));
    }

    Err(NSError::NotFound("cann't find system config service url".to_string()))
}




pub struct ZoneProvider {
    client : OnceCell<SystemConfigClient>,
    session_token: Option<String>,
    this_device: Option<DeviceInfo>,
    is_gateway:bool,
}

impl ZoneProvider {
    pub fn new(this_device: Option<&DeviceInfo>,session_token: Option<&String>,is_gateway:bool) -> Self {
        Self { 
            client:OnceCell::new(),
            session_token:session_token.map(|s|s.clone()),
            this_device:this_device.map(|d|d.clone()),
            is_gateway,
        }
    }

    async fn do_system_config_client_query(&self,client:&SystemConfigClient,name:&str) -> NSResult<NameInfo> {
        let device_info_path = sys_config_get_device_path(name) + "/info";
        let get_result = client.get(device_info_path.as_str()).await;
        if get_result.is_ok() {
            let (device_info_json,_) = get_result.unwrap();
            let device_info_value : Value = serde_json::from_str(device_info_json.as_str()).map_err(|e|{
                warn!("ZoneProvider parse device info for {} failed: {}",name,e);
                NSError::NotFound(format!("parse device info for {} failed: {}",name,e))
            })?;
            let ip = device_info_value.get("ip");
            if ip.is_some() {
                let ip = ip.unwrap();
                let ip_str = ip.as_str();
                if ip_str.is_none() {
                    return Err(NSError::NotFound(format!("ip for {} not found",name)));
                }

                let ip_str = ip_str.unwrap();
                let ip_addr = IpAddr::from_str(ip_str).map_err(|e|{
                    warn!("ZoneProvider resolve ip for {} failed: {}",name,e);
                    NSError::NotFound(format!("resolve ip for {} failed: {}",name,e))
                })?;

                return Ok(NameInfo::from_address(name,ip_addr));
            }
        }
        return Err(NSError::NotFound(format!("device info for {} not found",name)))
    }

}



#[async_trait]
impl NSProvider for ZoneProvider {
    fn get_id(&self) -> String {
        "zone provider".to_string()
    }

    async fn query(&self, name: &str,record_type:Option<&str>,from_ip:Option<IpAddr>) -> NSResult<NameInfo> {
        let record_type = record_type.unwrap_or("A");
        if record_type != "A"  {
            return Err(NSError::NotFound("only support A record now".to_string()));
        }

        if name.contains(".") {
            warn!("ZoneProvider only support device name resolve now, {} is not a device name",name);
            return Err(NSError::NotFound(format!("only support device name resolve now, {} is not a device name",name)));
        }

        let client = self.client.get();
        if client.is_some() {
            info!("ZoneProvider try resolve ip by system config service for {} ...",name);
            let client = client.unwrap();
            let name_info = self.do_system_config_client_query(&client,name).await;
            if name_info.is_ok() {
                info!("ZoneProvider resolve ip by system config service for {} success",name);
                return name_info;
            }
        } else {
            let zone_config = CURRENT_ZONE_CONFIG.get();
            if zone_config.is_none() {
                warn!("ZoneProvider cann't resolve ip for {},zone config not found",name);
                return Err(NSError::NotFound("zone config not found".to_string()));
            }
            let zone_config = zone_config.unwrap();
            let this_device = self.this_device.as_ref();
            let system_config_url = get_system_config_service_url(this_device, &zone_config,self.is_gateway).await;
            if system_config_url.is_ok() {
                let system_config_url = system_config_url.unwrap();
                info!("ZoneProvider try connect to system config service {} for resolve ip for {} ...",system_config_url.as_str(),name);
                let client = SystemConfigClient::new(Some(system_config_url.as_str()),self.session_token.as_deref());
                let name_info = self.do_system_config_client_query(&client, name).await;
                if name_info.is_ok() {
                    info!("ZoneProvider first resolve ip by system config service for {} success",name);
                    let set_result = self.client.set(client);
                    if set_result.is_err() {
                        warn!("ZoneProvider set system config client failed");
                    }
                    return name_info;
                } else {
                    warn!("ZoneProvider first resolve ip by system config service for {} failed",name);
                }
            }

            //if target device is ood, try resolve ip by ood info in zone config
            info!("ZoneProvider try resolve ip by ood info in zone config for {} ...",name);
            let ood_string = zone_config.get_ood_string(name);
            if ood_string.is_some() {
                let ood_info = DeviceInfo::new(ood_string.unwrap().as_str(),None);
                let ip = resolve_ood_ip_by_info(&ood_info,&zone_config).await;
                if ip.is_ok() {
                    return Ok(NameInfo::from_address(name,ip.unwrap()));
                }
            } 
        }

        Err(NSError::NotFound(format!("cann't resolve ip for {}",name)))
    }

    async fn query_did(&self, did: &str,fragment:Option<&str>,from_ip:Option<IpAddr>) -> NSResult<EncodedDocument> {
        let zone_config = CURRENT_ZONE_CONFIG.get();
        if zone_config.is_none() {
            return Err(NSError::NotFound("zone config not found".to_string()));
        }
        let zone_config = zone_config.unwrap();
        if did == zone_config.did.as_str() {
            let zone_value = serde_json::to_value(zone_config).unwrap();
            return Ok(EncodedDocument::JsonLd(zone_value));
        }

        Err(NSError::NotFound(format!("did {} not found",did)))
    }

}