#![allow(unused)]
use cyfs_gateway_lib::TunnelSelector;
use ::kRPC::*;
use std::{fmt::format, net::{IpAddr, Ipv4Addr}, result::Result};
use async_trait::async_trait;
use serde_json::{Value,json};
use name_lib::*;
use name_client::*;
use log::*;
use tokio::sync::Mutex;
use std::sync::Arc;
use std::collections::HashMap;
use serde::{Serialize,Deserialize};
use std::str::FromStr;
use lazy_static::lazy_static;

use crate::sn_db::{self, *};

#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct SNServerConfig {
    host:String,
    ip:String,
}


lazy_static!{
    static ref SN_SERVER_MAP:Arc<Mutex<HashMap<String,SNServer>>> = Arc::new(Mutex::new(HashMap::new()));
}

#[derive(Clone)]
pub struct SNServer {
    //ipaddress is the ip from update_op's ip_from
    all_device_info:Arc<Mutex<HashMap<String,(DeviceInfo,IpAddr)>>>,
    all_user_zone_config:Arc<Mutex<HashMap<String,String>>>,
    server_host:String,
    server_ip:IpAddr,
}

impl SNServer {
    pub fn new(server_config:Option<SNServerConfig>) -> Self {
        let conn = get_sn_db_conn();
        if conn.is_ok() {
            let conn = conn.unwrap();
            initialize_database(&conn);
        } else {
            error!("Failed to open sn_db.sqlite3");
            panic!("Failed to open sn_db.sqlite3");
        }

        let mut server_host = "web3.buckyos.io".to_string();
        let mut server_ip:IpAddr = IpAddr::V4(Ipv4Addr::new(127,0,0,1));
        if server_config.is_some() {
            let server_config = server_config.unwrap();
            server_host = server_config.host;
            server_ip = IpAddr::from_str(server_config.ip.as_str()).unwrap();
        } 

        SNServer {
            all_device_info:Arc::new(Mutex::new(HashMap::new())),
            all_user_zone_config:Arc::new(Mutex::new(HashMap::new())),
            server_host:server_host,
            server_ip:server_ip,
        }
    }

    pub async fn get_user_tls_cert(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        unimplemented!();
    }

    pub async fn check_username(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let username = req.params.get("username");
        if username.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, username is none".to_string()));
        }
        let username = username.unwrap().as_str();
        let conn = sn_db::get_sn_db_conn().map_err(|e|{
            error!("Failed to get sn_db_conn: {:?}",e);
            RPCErrors::ReasonError(e.to_string())
        })?;
        let username = username.unwrap();
        let ret = sn_db::is_user_exist(&conn, username).map_err(|e|{
            error!("Failed to check username: {:?}",e);
            RPCErrors::ReasonError(e.to_string())
        })?;
        let resp = RPCResponse::new(RPCResult::Success(json!({
            "valid":!ret 
        })),req.seq);
        return Ok(resp);
    }

    pub async fn check_active_code(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let active_code = req.params.get("active_code");
        if active_code.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, active_code is none".to_string()));
        }
        let active_code = active_code.unwrap().as_str();
        if active_code.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, active_code is none".to_string()));
        }
        let active_code = active_code.unwrap();
        let conn = sn_db::get_sn_db_conn().unwrap();
        let ret = sn_db::check_active_code(&conn, active_code);
        if ret.is_err() {
            return Err(RPCErrors::ReasonError(ret.err().unwrap().to_string()));
        }
        let valid = ret.unwrap();
        let resp = RPCResponse::new(RPCResult::Success(json!({
            "valid":valid 
        })),req.seq);
        return Ok(resp);
    }

    pub async fn register_user(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let user_name = req.params.get("user_name");
        let public_key = req.params.get("public_key");
        let active_code = req.params.get("active_code");
        let zone_config_jwt = req.params.get("zone_config");
        if user_name.is_none() || public_key.is_none() || active_code.is_none() || zone_config_jwt.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, user_name or public_key or active_code or zone_config (jwt) is none".to_string()));
        }
        let user_name = user_name.unwrap().as_str().unwrap();
        let public_key = public_key.unwrap().as_str().unwrap();
        let active_code = active_code.unwrap().as_str().unwrap();
        let zone_config_jwt = zone_config_jwt.unwrap().as_str().unwrap();

        let conn = sn_db::get_sn_db_conn().unwrap();
        let ret = sn_db::register_user(&conn, active_code, user_name, public_key, zone_config_jwt);
        if ret.is_err() {
            let err_str = ret.err().unwrap().to_string();
            warn!("Failed to register user {}: {:?}",user_name,err_str.as_str());
            return Err(RPCErrors::ParseRequestError(format!("Failed to register user: {}",err_str)));
        } 

        info!("user {} registered success, public_key: {}, active_code: {}",user_name,public_key,active_code);

        let resp = RPCResponse::new(RPCResult::Success(json!({
            "code":0 
        })),req.seq);
        return Ok(resp);
    }

    pub async fn register_device(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let user_name = req.params.get("user_name");
        let device_name = req.params.get("device_name");
        let device_did = req.params.get("device_did");
        let device_ip = req.params.get("device_ip");
        let device_info = req.params.get("device_info");

        if user_name.is_none() || device_name.is_none() || device_did.is_none() || device_ip.is_none() || device_info.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, user_name or device_name or device_did or device_ip or device_info is none".to_string()));
        }
        let user_name = user_name.unwrap().as_str().unwrap();
        let device_name = device_name.unwrap().as_str().unwrap();
        let device_did = device_did.unwrap().as_str().unwrap();
        let device_ip = device_ip.unwrap().as_str().unwrap();
        let device_info = device_info.unwrap().as_str().unwrap();

        let conn = sn_db::get_sn_db_conn().unwrap();
        let ret = sn_db::register_device(&conn, user_name, device_name, device_did, device_ip, device_info);
        if ret.is_err() {
            let err_str = ret.err().unwrap().to_string();
            warn!("Failed to register device {}_{}: {:?}",user_name,device_name,err_str.as_str());
            return Err(RPCErrors::ParseRequestError(format!("Failed to register device: {}",err_str)));
        }   

        info!("device {}_{} registered success",user_name,device_name);

        let resp = RPCResponse::new(RPCResult::Success(json!({
            "code":0 
        })),req.seq);
        return Ok(resp);
    }

    pub async fn update_device(&self, req:RPCRequest,ip_from:IpAddr) -> Result<RPCResponse,RPCErrors> {
        let device_info_json = req.params.get("device_info");
        let owner_id = req.params.get("owner_id");
        if owner_id.is_none() || device_info_json.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, owner_id or device_info is none".to_string()));
        }
        let owner_id = owner_id.unwrap().as_str();
        if owner_id.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, owner_id is none".to_string()));
        }
        let owner_id = owner_id.unwrap();
        let device_info_json = device_info_json.unwrap();
    

        let device_info = serde_json::from_value::<DeviceInfo>(device_info_json.clone()).map_err(|e|{
            error!("Failed to parse device info: {:?}",e);
            RPCErrors::ParseRequestError(e.to_string())
        })?;    

        info!("update {}_{} ==> {:?}",owner_id,device_info.hostname.clone(),device_info_json);

        let mut device_info_map = self.all_device_info.lock().await;
        let key = format!("{}_{}",owner_id,device_info.hostname.clone());
        device_info_map.insert(key.clone(), (device_info.clone(),ip_from));
        let conn = sn_db::get_sn_db_conn().unwrap();
        let ip_str = ip_from.to_string();
        sn_db::update_device_by_name(&conn, owner_id, &device_info.hostname.clone(), ip_str.as_str(), device_info_json.to_string().as_str());
        let resp = RPCResponse::new(RPCResult::Success(json!({
            "code":0 
        })),req.seq);

        info!("update device info done: for {}",key);
        return Ok(resp);
    }
    

    async fn get_device_info(&self, owner_id: &str,device_name: &str) -> Option<(DeviceInfo,IpAddr)> {
        let key = format!("{}_{}",owner_id,device_name);
        let mut device_info_map = self.all_device_info.lock().await;
        let device_info = device_info_map.get(&key);
        if device_info.is_none() {
            warn!("device info not found for {} in memory cache, try to query from db",key);
            let conn = sn_db::get_sn_db_conn().unwrap();
            let device_json = sn_db::query_device_by_name(&conn, owner_id, device_name).unwrap();
            if device_json.is_none() {
                warn!("device info not found for {} in db",key);
                return None;
            }
            let device_json = device_json.unwrap();
            let sn_ip = device_json.3;
            let sn_ip = IpAddr::from_str(sn_ip.as_str()).unwrap();
            let device_info_json:String = device_json.4;
            //info!("device info json: {}",device_info_json);
            let device_info = serde_json::from_str::<DeviceInfo>(device_info_json.as_str());
            if device_info.is_err() {
                warn!("failed to parse device info from db for {}: {:?}",key,device_info.err().unwrap());
                return None;
            }
            let device_info = device_info.unwrap();
            device_info_map.insert(key.clone(), (device_info.clone(),sn_ip));
            return Some((device_info.clone(),sn_ip));
        } else {
            return device_info.cloned();
        }
    }

    async fn get_user_zone_config(&self, username: &str) -> Option<String> {
        let mut user_zone_config_map = self.all_user_zone_config.lock().await;
        let zone_config = user_zone_config_map.get(username);
        if zone_config.is_none() {
            let conn = sn_db::get_sn_db_conn().unwrap();
            let user_info = sn_db::get_user_info(&conn, username).unwrap();
            if user_info.is_some() {
                let user_info = user_info.unwrap();
                user_zone_config_map.insert(username.to_string(), user_info.1.clone());
                return Some(user_info.1.clone());
            }
            warn!("zone config not found for [{}]",username);
            return None;
        } else {
            return zone_config.cloned();
        }
    }


    //get device info by device_name and owner_name
    pub async fn get_device(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let device_id = req.params.get("device_id");
        let owner_id = req.params.get("owner_id");
        if owner_id.is_none() || device_id.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, owner_id or device_info is none".to_string()));
        }
        let device_id = device_id.unwrap().as_str();
        let owner_id = owner_id.unwrap().as_str();
        if device_id.is_none() || owner_id.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, device_id or owner_id is none".to_string()));
        }
        let device_id = device_id.unwrap();
        let owner_id = owner_id.unwrap();
        let device_info = self.get_device_info(device_id, owner_id).await;
        if device_info.is_some() {
            let device_info = device_info.unwrap();
            let device_value = serde_json::to_value(device_info.0).map_err(|e|{
                warn!("Failed to parse device info: {:?}",e);
                RPCErrors::ReasonError(e.to_string())
            })?;
            return Ok(RPCResponse::new(RPCResult::Success(device_value),req.seq));
        }
         else {
            warn!("device info not found for {}_{}",owner_id,device_id);
            let device_json = serde_json::to_value(device_info.clone()).unwrap();
            return Ok(RPCResponse::new(RPCResult::Success(device_json),req.seq)); 
        }
  
    }
}

#[async_trait]
impl NSProvider for SNServer {
    fn get_id(&self) -> String {
        "sn_ns_provider".to_string()
    } 

    async fn query(&self, name: &str,record_type:Option<&str>,from_ip:Option<IpAddr>) -> NSResult<NameInfo> {
        info!("sn server dns process name query: {}, record_type: {:?}",name,record_type);
        let record_str = record_type.unwrap_or("A");
        let from_ip = from_ip.unwrap_or(self.server_ip);
        let mut is_support = false;
        if record_str == "A" || record_str == "AAAA" || record_str == "TXT" {
            is_support = true;
        }

        if !is_support {
            return Err(NSError::NotFound(format!("sn-server not support record type {}",record_str)));
        }

        let full_server_host = format!("{}.",self.server_host.as_str());
        if name == self.server_host || name == full_server_host {
            //返回当前服务器的地址
            let result_name_info = NameInfo::from_address(name, self.server_ip);
            return Ok(result_name_info);
        }
        //query A or AAAA record
        //端口映射方案: 如果用户存在 返回设备ood1的IP 
        //使用web3桥返连方案:如果用户存在和ood1都存在 返回当前服务器的IP 
        
        //query TXT record
        //如果用户存在，则返回用户的ZoneConfig
        let end_string = format!(".{}.",self.server_host.as_str());
        if name.ends_with(&end_string) {
            
            let sub_name = name[0..name.len()-end_string.len()].to_string();
            //split sub_name by "."
            let subs:Vec<&str> = sub_name.split(".").collect();
            let username = subs.last();
            if username.is_none() {
                return Err(NSError::NotFound(name.to_string()));
            }
            let username = username.unwrap();
            info!("sub zone {},enter sn serverquery: {}, record_type: {:?}",username,name,record_type);
            match record_str {
                "TXT" => {
                    let zone_config = self.get_user_zone_config(username).await;
                    if zone_config.is_some() {
                        let result_name_info = NameInfo::from_zone_config_str(name, zone_config.unwrap().as_str());
                        return Ok(result_name_info);
                    } else {
                        return Err(NSError::NotFound(name.to_string()));
                    }
                },
                "A" | "AAAA" => {
                    let device_info = self.get_device_info(username, "ood1").await;
                    if device_info.is_some() {
                        let (device_info,device_ip) = device_info.unwrap();
                        let mut address_vec:Vec<IpAddr> = Vec::new();
                        let device_report_ip = device_info.ip;
                        if device_info.is_wan_device() {

                            if device_report_ip.is_some() {
                                let device_report_ip = device_report_ip.unwrap();
                                match device_report_ip {
                                    IpAddr::V4(ip) => {
                                        if ip.is_private() {
                                            if from_ip == device_ip {
                                                info!("device {} is wan device and query from some lan, return lan_ip {} and device_ip {}",name,device_report_ip,device_ip);
                                                address_vec.push(device_report_ip);
                                                address_vec.push(device_ip);
                                            } else {
                                                info!("device {} is wan device with lan_ip, return device_ip {}",name,device_ip);
                                                address_vec.push(device_ip);
                                                address_vec.push(device_report_ip);
                                            }
                                        } else {
                                            info!("device {} is wan device with public_v4ip, return report ip {} ",name,device_report_ip);
                                            address_vec.push(device_report_ip);
                                        }
                                    }
                                    IpAddr::V6(ip) => {
                                        info!("device {} is wan device with v6, return report ip {} ",name,device_report_ip);
                                        address_vec.push(device_report_ip);
                                    }
                                }
                            } else {
                                info!("device {} is wan device without self-report ip, return device_ip {}",name,device_ip);
                                address_vec.push(device_ip);
                            }
                        } else {
                            if from_ip == device_ip  && device_report_ip.is_some() {
                                let device_report_ip = device_report_ip.unwrap();
                                info!("device {} is lan device and query from some lan, return self la_ip {} and sn_ip ",name,device_report_ip);
                                address_vec.push(device_report_ip);
                                address_vec.push(self.server_ip);
                            } else {
                                info!("device {} is lan device , return sn_ip",name);
                                address_vec.push(self.server_ip);
                                if device_report_ip.is_some() {
                                    let device_report_ip = device_report_ip.unwrap();
                                    address_vec.push(device_report_ip);
                                }
                            }
                        }

                        let result_name_info = NameInfo::from_address_vec(name, address_vec);
                        return Ok(result_name_info);
                    } else {
                        return Err(NSError::NotFound(name.to_string()));
                    }
                },
                _ => {
                    return Err(NSError::NotFound(format!("sn-server not support record type {}",record_str)));
                }
            }
            
        } else {
            return Err(NSError::NotFound(name.to_string()));
        }
    }

    async fn query_did(&self, did: &str,fragment:Option<&str>,from_ip:Option<IpAddr>) -> NSResult<EncodedDocument> {
        return Err(NSError::NotFound("sn-server not support did query".to_string()));
    }
}

#[async_trait]
impl kRPCHandler for SNServer {
    async fn handle_rpc_call(&self, req:RPCRequest,ip_from:IpAddr) -> Result<RPCResponse,RPCErrors> {
        match req.method.as_str() {
            "get_user_tls_cert" => {
                //get user tls cert
                return self.get_user_tls_cert(req).await;
            },
            "check_active_code" => {
                //check active code
                return self.check_active_code(req).await;
            },
            "check_username" => {
                //check username
                return self.check_username(req).await;
            },
            "register_user" => {
                //register user
                return self.register_user(req).await;
            },
            "register" => {
                //register device
                return self.register_device(req).await;
            },
            "update" => {
                //update device info
                return self.update_device(req,ip_from).await;
            },
            "get" => {
                //get device info
                return self.get_device(req).await;
            }
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}


#[async_trait]
impl TunnelSelector for SNServer {
    async fn select_tunnel_for_http_upstream(&self, req_host:&str,req_path:&str) -> Option<String> {
        let end_string = format!(".{}",self.server_host.as_str());
        if req_host.ends_with(&end_string) {
            let sub_name = req_host[0..req_host.len()-end_string.len()].to_string();
            //split sub_name by "."
            let subs:Vec<&str> = sub_name.split(".").collect();
            let username = subs.last();
            if username.is_none() {
                warn!("invalid username for sn tunnel selector {}",req_host);
                return None;
            }
            let username = username.unwrap();
            
            let device_info = self.get_device_info(username, "ood1").await;
            if device_info.is_some() {
                //info!("ood1 device info found for {} in sn server",username);
                //let device_did = device_info.unwrap().0.did;
                let device_did = device_info.unwrap().0.did;
                if device_did.is_some() {
                    let device_did = device_did.unwrap().replace(":", ".");
                    let result_str = format!("rtcp://{}",device_did.as_str());
                    //info!("select device {} for http upstream:{}",device_did.as_str(),result_str.as_str());
                    return Some(result_str);
                } else {
                    warn!("ood1 device did not found for {} in sn server",username);
                }
            } else {
                warn!("ood1 device info not found for {} in sn server",username);
            }
        }

        return None;
    }
}

pub async fn register_sn_server(server_id:&str, sn_server:SNServer) {
    let mut server_map = SN_SERVER_MAP.lock().await;
    server_map.insert(server_id.to_string(), sn_server);
}

pub async fn get_sn_server_by_id(server_id:&str) -> Option<SNServer> {
    let server_map = SN_SERVER_MAP.lock().await;
    let sn_server = server_map.get(server_id);
    if sn_server.is_none() {
        return None;
    }
    let sn_server = sn_server.unwrap();
    Some(sn_server.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_device_info() {
        let req_host = "home.lzc.web3.buckyos.io".to_string();
        let server_host = "web3.buckyos.io".to_string();
        let end_string = format!(".{}",server_host.as_str());
        if req_host.ends_with(&end_string) {
            let sub_name = req_host[0..req_host.len()-end_string.len()].to_string();
            //split sub_name by "."
            let subs:Vec<&str> = sub_name.split(".").collect();
            let username = subs.last();
            if username.is_none() {
                warn!("invalid username for sn tunnel selector {}",req_host);
                return;
            }
            let username = username.unwrap();
            println!("username: {}",username);
        }
    }
}
    
