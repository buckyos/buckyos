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

use std::net::{IpAddr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::str::FromStr;
use async_trait::async_trait;
use serde_json::{Value, json};
use bytes::Bytes;
use http::Method;
use http_body_util::{combinators::BoxBody, Full, BodyExt};
use cyfs_gateway_lib::{HttpServer, ServerError, ServerResult, StreamInfo, server_err, ServerErrorCode};
use url::{Url, form_urlencoded};

use crate::SYS_STORE;

// fn is_unicast_link_local_stable(ipv6: &Ipv6Addr) -> bool {
//     ipv6.segments()[0] == 0xfe80
// }

// //ipv4 first host resolve
// fn resolve_hostname(hostname: &str) -> Option<std::net::IpAddr> {
//     let addrs = (hostname, 0).to_socket_addrs().ok()?;
//     let mut ipv6_addr :Option<IpAddr> = None;
//     for addr in addrs {
//         if addr.is_ipv4() {
//             let ip = addr.ip();
//             match ip {
//                 IpAddr::V4(ipv4) => {
//                     return Some(IpAddr::V4(ipv4));
//                 },
//                 _ => {}
//             }

//         } else if addr.is_ipv6() {
//             let ip = addr.ip();
//             match ip {
//                 IpAddr::V6(ipv6) => {
//                     if !is_unicast_link_local_stable(&ipv6) {
//                         ipv6_addr = Some(IpAddr::V6(ipv6));
//                     }
//                 }
//                 _ => {}
//             }
//         }
//     }

//     if ipv6_addr.is_some() {
//         return Some(ipv6_addr.unwrap());
//     }

//     return None;
// }


// //ipv4 first local host resolve
// fn resolve_lan_hostname(hostname: &str) -> Option<std::net::IpAddr> {
//     let addrs = (hostname, 0).to_socket_addrs().ok()?;
//     let mut private_ipv6_addr :Option<IpAddr> = None;
//     for addr in addrs {
//         if addr.is_ipv4() {
//             let ip = addr.ip();
//             match ip {
//                 IpAddr::V4(ipv4) => {
//                     if ipv4.is_private() {
//                         return Some(IpAddr::V4(ipv4));
//                     }
//                 },
//                 _ => {}
//             }

//         } else if addr.is_ipv6() {
//             let ip = addr.ip();
//             match ip {
//                 IpAddr::V6(ipv6) => {
//                     if !is_unicast_link_local_stable(&ipv6) {
//                         private_ipv6_addr = Some(IpAddr::V6(ipv6));
//                     }
//                 },
//                 _ => {}
//             }
//         }
//     }

//     if private_ipv6_addr.is_some() {
//         return Some(private_ipv6_addr.unwrap());
//     }

//     return None;
// }

// async fn resolve_ood_ip_by_info(ood_info: &DeviceInfo,zone_config:&ZoneConfig) -> NSResult<IpAddr> {
//     if ood_info.ips.len() > 0 {
//         return Ok(ood_info.ips[0].clone());
//     }

//     let zone_short_name = zone_config.id.to_host_name();

//     if !ood_info.is_wan_device() {
//         let hostname = format!("{}.{}",ood_info.name.as_str(),zone_short_name.as_str());
//         let addr = resolve_lan_hostname(hostname.as_str());
//         if addr.is_some() {
//             return Ok(addr.unwrap());
//         }

//         let hostname = format!("{}.local",ood_info.name.as_str());
//         let addr = resolve_lan_hostname(hostname.as_str());
//         if addr.is_some() {
//             return Ok(addr.unwrap());
//         }
//     }

//     //try resolve by HTTP-SN
//     if let Some(sn_url) = zone_config.get_sn_api_url() {
//         info!("try resolve ood {} ip by sn: {}",ood_info.name.clone(),sn_url);
//         let owner_id = zone_config.owner.clone();
//         if owner_id.is_valid() {
//             let device_info = sn_get_device_info(
//                 sn_url.as_str(),
//                 None,
//                 &owner_id.id.to_string(),
//                 ood_info.name.as_str()
//             ).await;

//             if let Ok(device_info) = device_info {
//                 if !device_info.ips.is_empty() {
//                     return Ok(device_info.ips[0].clone());
//                 }

//                 if !device_info.all_ip.is_empty() {
//                     return Ok(device_info.all_ip[0].clone());
//                 }
//             }
//         }
//     }

//     Err(NSError::Failed("cann't resolve ip for device".to_string()))
// }


//TODO: 需要更系统性的思考如何得到 各种service的URL
// pub async fn get_system_config_service_url(this_device:Option<&DeviceInfo>,zone_config:&ZoneConfig,is_gateway:bool) -> NSResult<String> {
//     if this_device.is_none() {
//         return Ok(String::from("http://127.0.0.1:3200/kapi/system_config"));
//     }

//     let this_device = this_device.unwrap();
//     let ood_string = zone_config.get_ood_desc_string(this_device.name.as_str());
//     if ood_string.is_some() {
//         return Ok(String::from("http://127.0.0.1:3200/kapi/system_config"));
//     }

//     //this device is not ood, looking best ood for system config service
//     let ood_info_str = zone_config.select_same_subnet_ood(this_device);
//     if ood_info_str.is_some() {
//         let ood_info = DeviceInfo::new(ood_info_str.unwrap().as_str(),this_device.id.clone());
//         info!("try connect to same subnet ood: {}",ood_info.name);
//         let ood_ip = resolve_ood_ip_by_info(&ood_info,zone_config).await;
//         if ood_ip.is_ok() {
//             let ood_ip = ood_ip.unwrap();
//             let server_url = format!("http://{}:3200/kapi/system_config",ood_ip);
//             return Ok(server_url);
//         }
//     } 

//     let ood_info_str = zone_config.select_wan_ood();
//     if ood_info_str.is_some() {
//         //try connect to wan ood
//         let ood_info = DeviceInfo::new(ood_info_str.unwrap().as_str(),this_device.id.clone());
//         info!("try connect to wan ood: {}",ood_info.name);
//         let ood_ip = resolve_ood_ip_by_info(&ood_info,zone_config).await;
//         if ood_ip.is_ok() {
//             let ood_ip = ood_ip.unwrap();
//             let server_url = format!("http://{}:3200/kapi/system_config",ood_ip);
//             return Ok(server_url);
//         }
//     }

//     if !is_gateway {
//         //connect to local cyfs_gateway,local cyfs-gateway will use tunnel connect to ood
//         warn!("cann't connect to ood directly, try connect to system config service by local cyfs_gateway");
//         return Ok(String::from("http://127.0.0.1:3180/kapi/system_config"));
//     }

//     Err(NSError::NotFound("cann't find system config service url".to_string()))
// }

// lazy_static! {
//     pub static ref ZONE_PROVIDER: ZoneProvider = ZoneProvider::new();
// }

#[derive(Clone)]
pub struct ZoneDidResolver {}

impl ZoneDidResolver {
    pub fn new() -> Self {
        Self {}
    }

    async fn load_zone_config_json(&self) -> NSResult<String> {
        let store = SYS_STORE.lock().await;
        let zone_config = store.get("boot/config".to_string()).await.map_err(|e| {
            warn!("ZoneDidResolver get zone config failed: {}", e);
            NSError::Failed(format!("get zone config failed: {}", e))
        })?;
        drop(store);

        zone_config.ok_or_else(|| NSError::NotFound("zone config not set".to_string()))
    }

    async fn load_device_doc(&self, device_id: &str) -> NSResult<(DeviceConfig, String)> {
        let obj_path = format!("devices/{}/doc", device_id);
        let store = SYS_STORE.lock().await;
        let obj_config_str = store.get(obj_path.clone()).await.map_err(|e| {
            warn!("ZoneDidResolver get {} failed: {}", obj_path, e);
            NSError::Failed(format!("get {} failed: {}", obj_path, e))
        })?;
        drop(store);

        let obj_config_str =
            obj_config_str.ok_or_else(|| NSError::NotFound(format!("device {} not found", device_id)))?;

        let encoded_doc = EncodedDocument::from_str(obj_config_str.clone()).map_err(|e| {
            warn!("ZoneDidResolver parse device config failed: {}", e);
            NSError::Failed(format!("parse device config failed: {}", e))
        })?;
        let device_config: DeviceConfig = DeviceConfig::decode(&encoded_doc, None).map_err(|e| {
            warn!("ZoneDidResolver decode device config failed: {}", e);
            NSError::Failed(format!("decode device config failed: {}", e))
        })?;

        Ok((device_config, obj_config_str))
    }

    async fn load_owner_doc(&self, owner_id: &str) -> NSResult<(OwnerConfig, String)> {
        let obj_path = format!("users/{}/doc", owner_id);
        let store = SYS_STORE.lock().await;
        let obj_config_str = store.get(obj_path.clone()).await.map_err(|e| {
            warn!("ZoneDidResolver get {} failed: {}", obj_path, e);
            NSError::Failed(format!("get {} failed: {}", obj_path, e))
        })?;
        drop(store);

        let obj_config_str =
            obj_config_str.ok_or_else(|| NSError::NotFound(format!("owner {} not found", owner_id)))?;

        let encoded_doc = EncodedDocument::from_str(obj_config_str.clone()).map_err(|e| {
            warn!("ZoneDidResolver parse owner config failed: {}", e);
            NSError::Failed(format!("parse owner config failed: {}", e))
        })?;
        let owner_config: OwnerConfig = OwnerConfig::decode(&encoded_doc, None).map_err(|e| {
            warn!("ZoneDidResolver decode owner config failed: {}", e);
            NSError::Failed(format!("decode owner config failed: {}", e))
        })?;

        Ok((owner_config, obj_config_str))
    }

    async fn load_device_info(&self, name: &str) -> NSResult<DeviceInfo> {
        let obj_path = format!("devices/{}/info", name);
        let store = SYS_STORE.lock().await;
        let device_info_str = store.get(obj_path.clone()).await.map_err(|e| {
            warn!("ZoneDidResolver get {} failed: {}", obj_path, e);
            NSError::Failed(format!("get {} failed: {}", obj_path, e))
        })?;
        drop(store);

        let device_info_str = device_info_str
            .ok_or_else(|| NSError::NotFound(format!("device info for {} not found", name)))?;

        serde_json::from_str::<DeviceInfo>(&device_info_str).map_err(|e| {
            warn!("ZoneDidResolver parse device info failed: {}", e);
            NSError::Failed(format!("parse device info failed: {}", e))
        })
    }

    async fn do_query_did(&self, did_str: &str, typ: Option<String>) -> NSResult<String> {
        if DID::is_did(did_str) {
            if let Ok((_device_config, obj_config_str)) = self.load_device_doc(did_str).await {
                info!("zone_provider resolve did {} => {}", did_str, obj_config_str.as_str());
                return Ok(obj_config_str);
            }

            if let Ok((_owner_config, obj_config_str)) = self.load_owner_doc(did_str).await {
                info!("zone_provider resolve did {} => {}", did_str, obj_config_str.as_str());
                return Ok(obj_config_str);
            }

            return Err(NSError::NotFound(format!("did {} not found", did_str)));
        } else {
            match did_str {
                "self" => {
                    let zone_config_str = self.load_zone_config_json().await?;
                    return Ok(zone_config_str);
                }
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
                    if let Ok((_device_config, obj_config_str)) = self.load_device_doc(did_str).await {
                        info!("zone_provider resolve name {} => {}", did_str, obj_config_str.as_str());
                        return Ok(obj_config_str);
                    }

                    if let Ok((_owner_config, obj_config_str)) = self.load_owner_doc(did_str).await {
                        info!("zone_provider resolve name {} => {}", did_str, obj_config_str.as_str());
                        return Ok(obj_config_str);
                    }

                    warn!("ZoneProvider resolve name {} failed", did_str);
                    return Err(NSError::NotFound(format!("name {} not found", did_str)));
                }
            }
        }
    }

    async fn do_query_info(&self, name: &str) -> NSResult<String> {
        let device_info = self.load_device_info(name).await?;
        serde_json::to_string(&device_info).map_err(|e| {
            warn!("ZoneDidResolver serialize device info failed: {}", e);
            NSError::Failed(format!("serialize device info failed: {}", e))
        })
    }

    // async fn do_system_config_client_query(&self, name: &str) -> NSResult<NameInfo> {
    //     let device_info = self.load_device_info(name).await?;

    //     if device_info.ips.len() > 0 {
    //         return Ok(NameInfo::from_address(name, device_info.ips[0].clone()));
    //     }

    //     if device_info.all_ip.len() > 0 {
    //         return Ok(NameInfo::from_address(name, device_info.all_ip[0].clone()));
    //     }

    //     return Err(NSError::NotFound(format!("device info for {} not found", name)));
    // }
}



// #[async_trait]
// impl NsProvider for ZoneProvider {
//     fn get_id(&self) -> String {
//         "zone provider".to_string()
//     }

//     async fn query(&self, name: &str,record_type:Option<RecordType>,from_ip:Option<IpAddr>) -> NSResult<NameInfo> {
//         let record_type = record_type.unwrap_or_default();
//         if record_type != RecordType::A  {
//             return Err(NSError::NotFound("only support A record now".to_string()));
//         }

//         if name.contains(".") {
//             warn!("ZoneProvider only support device name resolve now, {} is not a device name",name);
//             return Err(NSError::NotFound(format!("only support device name resolve now, {} is not a device name",name)));
//         }

//         let name_info = self.do_system_config_client_query(name).await;
//         if name_info.is_ok() {
//             info!("ZoneProvider resolve ip by system config service for {} success",name);
//             return name_info;
//         }


//         //if target device is ood, try resolve ip by ood info in zone config
//         info!("ZoneProvider try resolve ip by ood info in zone config for {} ...",name);
//         let runtime = get_buckyos_api_runtime().unwrap();
//         let zone_config = runtime.get_zone_config();
//         if zone_config.is_none() {
//             return Err(NSError::NotFound("zone config not found".to_string()));
//         }
//         let zone_config = zone_config.unwrap();
//         let ood_desc = zone_config.oods.iter().find(|ood| ood.name == name);
//         if let Some(ood_desc) = ood_desc {
//             //TODO: 需要更系统性的思考如何得到 devi
//             let ood_info = DeviceInfo::new(ood_desc,DID::new("dns","ood"));
//             if let Ok(ip) = resolve_ood_ip_by_info(&ood_info,&zone_config).await {
//                 return Ok(NameInfo::from_address(name,ip));
//             }
//         } 
        
//         Err(NSError::NotFound(format!("cann't resolve ip for {}",name)))
//     }

//     async fn query_did(&self, did: &DID,fragment:Option<&str>,from_ip:Option<IpAddr>) -> NSResult<EncodedDocument> {
//         let runtime = get_buckyos_api_runtime().unwrap();
//         let zone_config = runtime.get_zone_config();
//         if zone_config.is_none() {
//             return Err(NSError::NotFound("zone config not found".to_string()));
//         }
//         let zone_config = zone_config.unwrap();
//         if did == &zone_config.id {
//             let zone_value = serde_json::to_value(zone_config).unwrap();
//             return Ok(EncodedDocument::JsonLd(zone_value));
//         }

//         Err(NSError::NotFound(format!("did {} not found",did.to_host_name())))
//     }

// }

#[async_trait]
impl HttpServer for ZoneDidResolver {
    async fn serve_request(&self, req: http::Request<BoxBody<Bytes, ServerError>>, info: StreamInfo) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {

        // helper for building CORS friendly JSON response
        let build_resp = |body: String| -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
            Ok(http::Response::builder()
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(http::header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
                .header(http::header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
                .body(
                    BoxBody::new(
                        Full::new(Bytes::from(body))
                            .map_err(|never: std::convert::Infallible| -> ServerError { match never {} })
                            .boxed(),
                    ),
                )
                .map_err(|e| server_err!(ServerErrorCode::InvalidData, "Failed to build response: {}", e))?)
        };

        // CORS 预检
        if *req.method() == Method::OPTIONS {
            return Ok(http::Response::builder()
                .status(http::StatusCode::NO_CONTENT)
                .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(http::header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
                .header(http::header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
                .body(
                    BoxBody::new(
                        Full::new(Bytes::from_static(b""))
                            .map_err(|never: std::convert::Infallible| -> ServerError { match never {} })
                            .boxed(),
                    ),
                )
                .map_err(|e| server_err!(ServerErrorCode::InvalidData, "Failed to build response: {}", e))?);
        }

        if *req.method() != Method::GET {
            return Err(server_err!(ServerErrorCode::BadRequest, "Method not allowed"));
        }

        // GET https://resolver.example.com/1.0/identifiers/did:dev:abcdefg?type=doc_type
        let path = req.uri().path().to_string();
        if path.starts_with("/1.0/identifiers/") {
            let did_str = path.trim_start_matches("/1.0/identifiers/").to_string();
            if did_str.is_empty() {
                return Err(server_err!(ServerErrorCode::BadRequest, "invalid did in path"));
            }

            // parse optional `type` query parameter
            let typ = req
                .uri()
                .query()
                .and_then(|q| {
                    form_urlencoded::parse(q.as_bytes())
                        .find(|(k, _)| k == "type")
                        .map(|(_, v)| v.into_owned())
                });

            let did_doc = self
                .do_query_did(did_str.as_str(), typ)
                .await
                .map_err(|e| server_err!(ServerErrorCode::InvalidData, "query did failed: {}", e))?;

            return build_resp(did_doc);
        }

        // GET http://{did_host_name}/.well-known/{doc_type}.json
        if path.starts_with("/.well-known/") && path.ends_with(".json") {
            // pick host from URI first, fallback to Host header
            let host = req
                .uri()
                .host()
                .map(|v| v.to_string())
                .or_else(|| {
                    req.headers()
                        .get(http::header::HOST)
                        .and_then(|v| v.to_str().ok())
                        .map(|v| v.split(':').next().unwrap_or(v).to_string())
                });

            if host.is_none() {
                return Err(server_err!(ServerErrorCode::BadRequest, "host not found"));
            }
            let host = host.unwrap();

            // doc_type 是文件名（去掉.json）
            let doc_type = path
                .trim_start_matches("/.well-known/")
                .trim_end_matches(".json")
                .to_string();

            // 将 host 转换为 DID:web 形式，若本身已是 DID 字符串则直接使用
            let did_str = if DID::is_did(host.as_str()) {
                host.clone()
            } else {
                format!("did:web:{}", host)
            };

            let did_doc = self
                .do_query_did(did_str.as_str(), Some(doc_type))
                .await
                .map_err(|e| server_err!(ServerErrorCode::InvalidData, "query did failed: {}", e))?;

            return build_resp(did_doc);
        }

        return Err(server_err!(ServerErrorCode::BadRequest, "Method not allowed"));
    }

    fn id(&self) -> String {
        "zone-did-resolver".to_string()
    }

    fn http_version(&self) -> http::Version {
        http::Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}


// #[async_trait]
// impl RPCHandler for ZoneProvider {
//     async fn handle_http_get(&self, req_path:&str,ip_from:IpAddr) -> std::result::Result<String,RPCErrors> {
//         // Check if the path contains a "resolve" folder and extract the filename after it
//         //url like https://dev-resolver.example.com/1.0/identifiers/did:dev:abcdefg
//         if req_path.starts_with("/1.0/identifiers/") {
           
//             let parts: Vec<&str> = req_path.split("/").collect();
//             let did_str = parts[parts.len() - 1];
//             info!("ZoneProvider do_query_did handle http get for {}",did_str);
//             return self.do_query_did(did_str,None).await.map_err(|e|{
//                 warn!("ZoneProvider query did failed: {}",e);
//                 RPCErrors::ReasonError(e.to_string())
//             });
//         }
//         // GET https://resolver.example.com/did-query?name=alice&type=user
//         if req_path.starts_with("/did-query") {
  
//             let base = "http://localhost";
//             let full_url = format!("{}{}", base, req_path);
//             let parsed_url = Url::parse(&full_url);
//             if parsed_url.is_err() {
//                 return Err(RPCErrors::ReasonError("invalid url".to_string()));
//             }
//             let parsed_url = parsed_url.unwrap();
//             let query_pairs = parsed_url.query_pairs().collect::<std::collections::HashMap<_, _>>();

//             // 提取 name 和 type 参数
//             info!("ZoneProvider did-query handle http get for {} with query: {:?}",req_path,query_pairs);
//             let name = query_pairs.get("name").map(|v| v.to_string());
//             let typ = query_pairs.get("type").map(|v| v.to_string());

//             if name.is_none() {
//                 warn!("ZoneProvider did-query handle http get for {} with missing name or type parameter",req_path);
//                 return Err(RPCErrors::ReasonError("missing name or type parameter".to_string()));
//             }

//             return self.do_query_did(name.unwrap().as_str(),typ).await.map_err(|e|{
//                 warn!("ZoneProvider did-query failed: {}",e);
//                 RPCErrors::ReasonError(e.to_string())
//             });
//         }
        
//         return Err(RPCErrors::UnknownMethod(req_path.to_string()));
//     }
//     async fn handle_rpc_call(&self, req:RPCRequest,ip_from:IpAddr) -> std::result::Result<RPCResponse,RPCErrors> {
//         return Err(RPCErrors::UnknownMethod(req.method));
//     }
// }
