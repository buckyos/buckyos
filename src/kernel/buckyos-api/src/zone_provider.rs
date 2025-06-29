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
use name_lib::*;
use name_client::*;

use std::net::{IpAddr, Ipv6Addr,SocketAddr,ToSocketAddrs};
use std::str::FromStr;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::{Value,json};
use crate::*;
use ::kRPC::*;
use lazy_static::lazy_static;
use std::sync::Mutex;


fn is_unicast_link_local_stable(ipv6: &Ipv6Addr) -> bool {
    ipv6.segments()[0] == 0xfe80
}

//ipv4 first host resolve
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


//ipv4 first local host resolve
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

async fn resolve_ood_ip_by_info(ood_info: &DeviceInfo,zone_config:&ZoneConfig) -> NSResult<IpAddr> {
    if ood_info.ip.is_some() {
        return Ok(ood_info.ip.unwrap().clone());
    }

    let zone_short_name = zone_config.get_zone_short_name();
    

    if !ood_info.is_wan_device() {
        let hostname = format!("{}-{}",zone_short_name.as_str(),ood_info.name.as_str());
        let addr = resolve_lan_hostname(hostname.as_str());
        if addr.is_some() {
            return Ok(addr.unwrap());
        }

        let hostname = format!("{}-{}.local",zone_short_name.as_str(),ood_info.name.as_str());
        let addr = resolve_lan_hostname(hostname.as_str());
        if addr.is_some() {
            return Ok(addr.unwrap());
        }
    }

    //try resolve by HTTP-SN
    let sn_url = zone_config.get_sn_api_url();
    if sn_url.is_some() {
        let sn_url = sn_url.unwrap();
        info!("try resolve ood {} ip by sn: {}",ood_info.name.clone(),sn_url);
        let device_info = sn_get_device_info(sn_url.as_str(),None,
            zone_config.get_zone_short_name().as_str(),ood_info.name.as_str()).await;

        if device_info.is_ok() {
            let device_info = device_info.unwrap();
            return Ok(device_info.ip.unwrap());
        }
    }

    Err(NSError::Failed("cann't resolve ip for device".to_string()))
}


//TODO: 需要更系统性的思考如何得到 各种service的URL
pub async fn get_system_config_service_url(this_device:Option<&DeviceInfo>,zone_config:&ZoneConfig,is_gateway:bool) -> NSResult<String> {
    if this_device.is_none() {
        return Ok(String::from("http://127.0.0.1:3200/kapi/system_config"));
    }

    let this_device = this_device.unwrap();
    let ood_string = zone_config.get_ood_desc_string(this_device.name.as_str());
    if ood_string.is_some() {
        return Ok(String::from("http://127.0.0.1:3200/kapi/system_config"));
    }

    //this device is not ood, looking best ood for system config service
    let ood_info_str = zone_config.select_same_subnet_ood(this_device);
    if ood_info_str.is_some() {
        let ood_info = DeviceInfo::new(ood_info_str.unwrap().as_str(),this_device.id.clone());
        info!("try connect to same subnet ood: {}",ood_info.name);
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
        let ood_info = DeviceInfo::new(ood_info_str.unwrap().as_str(),this_device.id.clone());
        info!("try connect to wan ood: {}",ood_info.name);
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

lazy_static! {
    pub static ref ZONE_PROVIDER: ZoneProvider = ZoneProvider::new();
}

#[derive(Clone)]
pub struct ZoneProvider {   
    did_cache:Arc<RwLock<HashMap<String,String>>>,
}

impl ZoneProvider {
    pub fn new() -> Self {
        let mut init_hash_map = HashMap::new();
        let runtime = get_buckyos_api_runtime().unwrap();

        let zone_config = runtime.get_zone_config();
        if zone_config.is_some() {
            let zone_config = zone_config.unwrap();
            let zone_config_str = serde_json::to_string_pretty(&zone_config).unwrap();
            init_hash_map.insert(zone_config.id.to_string(),zone_config_str);
        }

        if runtime.device_config.is_some() {
            let device_config = runtime.device_config.as_ref().unwrap();
            let device_config_str = serde_json::to_string_pretty(&device_config).unwrap();
            init_hash_map.insert(device_config.id.to_string(),device_config_str);
        }

        Self { 
            did_cache:Arc::new(RwLock::new(init_hash_map)),
        }
    }


    async fn do_query_did(&self,did_str:&str) -> NSResult<String> {
        if DID::is_did(did_str) {
            let did = DID::from_str(did_str);
            if did.is_ok() {
                let did = did.unwrap();
                let cache = self.did_cache.read().await;
                let did_doc = cache.get(did_str);
                if did_doc.is_some() {
                    let did_doc_str = did_doc.unwrap().to_string();
                    info!("zone_provider resolve did {} => {}",did_str,did_doc_str.as_str());
                    return Ok(did_doc_str);
                } 
            }
            return Err(NSError::NotFound(format!("did {} not found",did_str)));
        } else {
            match did_str {
                // "this_zone" => {
                //     let zone_config = CURRENT_ZONE_CONFIG.get();
                //     if zone_config.is_none() {
                //         return Err(NSError::NotFound("current zone config not found".to_string()));
                //     }
                //     let zone_config = zone_config.unwrap();
                //     return Ok(serde_json::to_string(&zone_config).unwrap());
                // },
                // "this_device" => {
                //     let device_config = CURRENT_DEVICE_CONFIG.get();
                //     if device_config.is_none() {
                //         return Err(NSError::NotFound("current device config not found".to_string()));
                //     }
                //     let device_config = device_config.unwrap();
                //     return Ok(serde_json::to_string(&device_config).unwrap());
                // },
                // "owner" => {
                //     // let owner_config = CURRENT_USER_CONFIG.get();
                //     // if owner_config.is_none() {
                //     //     return Err(NSError::NotFound("current owner config not found".to_string()));
                //     // }
                //     // let owner_config = owner_config.unwrap();
                //     // return Ok(serde_json::to_string(&owner_config).unwrap());
                //     unimplemented!()
                // },
                _ => {
                    let runtime = get_buckyos_api_runtime().map_err(|e|{
                        warn!("ZoneProvider get buckyos api runtime failed: {}",e);
                        NSError::NotFound(format!("get buckyos api runtime failed: {}",e))
                    })?;
        
                    let system_config_client = runtime.get_system_config_client().await
                        .map_err(|e|{
                            warn!("ZoneProvider get system config client failed: {}",e);
                            NSError::NotFound(format!("get system config client failed: {}",e))
                        })?;
                        
                    let obj_path = format!("devices/{}/doc",did_str);
                    let obj_config_str = system_config_client.get(obj_path.as_str()).await;
                    if obj_config_str.is_ok() {

                        let obj_config_str = obj_config_str.unwrap().0;
                        let encoded_doc = EncodedDocument::from_str(obj_config_str.clone()).map_err(|e|{
                            warn!("ZoneProvider parse device config failed: {}",e);
                            NSError::Failed(format!("parse device config failed: {}",e))
                        })?;
                        let device_config:DeviceConfig = DeviceConfig::decode(&encoded_doc,None).map_err(|e|{
                            warn!("ZoneProvider parse device config failed: {}",e);
                            NSError::Failed(format!("parse device config failed: {}",e))
                        })?;
                        let mut cache = self.did_cache.write().await;
                        cache.insert(device_config.id.to_string(),obj_config_str.clone());
                        drop(cache);
                        info!("zone_provider resolve name {} => {}",did_str,obj_config_str.as_str());
                        return Ok(obj_config_str);
                    }

                    let obj_path = format!("users/{}/doc",did_str);
                    let obj_config_str = system_config_client.get(obj_path.as_str()).await;
                    if obj_config_str.is_ok() {
                        let obj_config_str = obj_config_str.unwrap();
                        let encoded_doc = EncodedDocument::from_str(obj_config_str.0.clone()).map_err(|e|{
                            warn!("ZoneProvider parse owner config failed: {}",e);
                            NSError::Failed(format!("parse owner config failed: {}",e))
                        })?;
                        let owner_config:OwnerConfig = OwnerConfig::decode(&encoded_doc,None).map_err(|e|{
                            warn!("ZoneProvider parse owner config failed: {}",e);
                            NSError::Failed(format!("parse owner config failed: {}",e))
                        })?;

                        let mut cache = self.did_cache.write().await;
                        cache.insert(owner_config.id.to_string(),obj_config_str.0.clone());
                        drop(cache);
                        info!("zone_provider resolve name {} => {}",did_str,obj_config_str.0.as_str());
                        return Ok(obj_config_str.0);
                    }
                    
                    warn!("ZoneProvider resolve name {} failed",did_str);
                    return Err(NSError::NotFound(format!("name {} not found",did_str)));
                }
            }    
        }
    }

    async fn do_query_info(&self,name:&str) -> NSResult<String> {
        let runtime = get_buckyos_api_runtime()
            .map_err(|e|{
                warn!("ZoneProvider get buckyos api runtime failed: {}",e);
                NSError::NotFound(format!("get buckyos api runtime failed: {}",e))
            })?;

        let control_panel_client = runtime.get_control_panel_client().await
            .map_err(|e|{
                warn!("ZoneProvider get control panel client failed: {}",e);
                NSError::NotFound(format!("get control panel client failed: {}",e))
            })?;
        
        let device_info = control_panel_client.get_device_info(name).await
            .map_err(|e|{
                warn!("ZoneProvider get device info failed: {}",e);
                NSError::NotFound(format!("get device info failed: {}",e))
            })?;

        let device_info_str = serde_json::to_string(&device_info).unwrap();
        return Ok(device_info_str);
    }

    async fn do_system_config_client_query(&self,name:&str) -> NSResult<NameInfo> {
        let runtime = get_buckyos_api_runtime()
            .map_err(|e|{
                warn!("ZoneProvider get buckyos api runtime failed: {}",e);
                NSError::NotFound(format!("get buckyos api runtime failed: {}",e))
            })?;

        let control_panel_client = runtime.get_control_panel_client().await
            .map_err(|e|{
                warn!("ZoneProvider get control panel client failed: {}",e);
                NSError::NotFound(format!("get control panel client failed: {}",e))
            })?;
        
        let device_info = control_panel_client.get_device_info(name).await
            .map_err(|e|{
                warn!("ZoneProvider get device info failed: {}",e);
                NSError::NotFound(format!("get device info failed: {}",e))
            })?;

        let ip = device_info.ip;
        if ip.is_some() {
            return Ok(NameInfo::from_address(name,ip.unwrap()));
        }

        return Err(NSError::NotFound(format!("device info for {} not found",name)))
    }

}



#[async_trait]
impl NsProvider for ZoneProvider {
    fn get_id(&self) -> String {
        "zone provider".to_string()
    }

    async fn query(&self, name: &str,record_type:Option<RecordType>,from_ip:Option<IpAddr>) -> NSResult<NameInfo> {
        let record_type = record_type.unwrap_or_default();
        if record_type != RecordType::A  {
            return Err(NSError::NotFound("only support A record now".to_string()));
        }

        if name.contains(".") {
            warn!("ZoneProvider only support device name resolve now, {} is not a device name",name);
            return Err(NSError::NotFound(format!("only support device name resolve now, {} is not a device name",name)));
        }

        let name_info = self.do_system_config_client_query(name).await;
        if name_info.is_ok() {
            info!("ZoneProvider resolve ip by system config service for {} success",name);
            return name_info;
        }


        //if target device is ood, try resolve ip by ood info in zone config
        info!("ZoneProvider try resolve ip by ood info in zone config for {} ...",name);
        let runtime = get_buckyos_api_runtime().unwrap();
        let zone_config = runtime.get_zone_config();
        if zone_config.is_none() {
            return Err(NSError::NotFound("zone config not found".to_string()));
        }
        let zone_config = zone_config.unwrap();
        let ood_string = zone_config.get_ood_desc_string(name);
        if ood_string.is_some() {
            //TODO: 需要更系统性的思考如何得到 devi
            let ood_info = DeviceInfo::new(ood_string.unwrap().as_str(),DID::new("dns","ood"));
            let ip = resolve_ood_ip_by_info(&ood_info,&zone_config).await;
            if ip.is_ok() {
                return Ok(NameInfo::from_address(name,ip.unwrap()));
            }
        } 
        
        Err(NSError::NotFound(format!("cann't resolve ip for {}",name)))
    }

    async fn query_did(&self, did: &DID,fragment:Option<&str>,from_ip:Option<IpAddr>) -> NSResult<EncodedDocument> {
        let runtime = get_buckyos_api_runtime().unwrap();
        let zone_config = runtime.get_zone_config();
        if zone_config.is_none() {
            return Err(NSError::NotFound("zone config not found".to_string()));
        }
        let zone_config = zone_config.unwrap();
        if did == &zone_config.id {
            let zone_value = serde_json::to_value(zone_config).unwrap();
            return Ok(EncodedDocument::JsonLd(zone_value));
        }

        Err(NSError::NotFound(format!("did {} not found",did.to_host_name())))
    }

}


#[async_trait]
impl InnerServiceHandler for ZoneProvider {
    async fn handle_http_get(&self, req_path:&str,ip_from:IpAddr) -> std::result::Result<String,RPCErrors> {
        // Check if the path contains a "resolve" folder and extract the filename after it
        if req_path.contains("/resolve/") {
            let parts: Vec<&str> = req_path.split("/resolve/").collect();
            if parts.len() > 1 && !parts[1].is_empty() {
                let domain_name = parts[1];
                debug!("ZoneProvider trying to resolve domain: {}", domain_name);
                if domain_name.ends_with("/info") {
                    let info = self.do_query_info(domain_name).await.map_err(|e|{
                        warn!("ZoneProvider query object info failed: {}",e);
                        RPCErrors::ReasonError(e.to_string())
                    })?;
                    return Ok(info);
         
                } else {
                    let did_doc = self.do_query_did(domain_name).await.map_err(|e|{
                        warn!("ZoneProvider query object did-document failed: {}",e);
                        RPCErrors::ReasonError(e.to_string())
                    })?;

                    let did_doc_string = did_doc.to_string();
                    return Ok(did_doc_string);
                }
            }
        }
        return Err(RPCErrors::UnknownMethod(req_path.to_string()));
    }
    async fn handle_rpc_call(&self, req:RPCRequest,ip_from:IpAddr) -> std::result::Result<RPCResponse,RPCErrors> {
        return Err(RPCErrors::UnknownMethod(req.method));
    }
}

