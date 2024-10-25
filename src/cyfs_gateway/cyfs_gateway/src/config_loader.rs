use std::collections::HashMap;
use cyfs_gateway_lib::DNSServerConfig;
use cyfs_gateway_lib::DispatcherConfig;
use cyfs_gateway_lib::ServerConfig;
use cyfs_gateway_lib::WarpServerConfig;
use cyfs_sn::*;
use cyfs_warp::register_inner_service_builder;
use url::Url;
use serde_json::from_value;
use log::*;
pub struct GatewayConfig {
    pub dispatcher : HashMap<Url,DispatcherConfig>,
    pub servers : HashMap<String,ServerConfig>,
    //tunnel_builder_config : HashMap<String,TunnelBuilderConfig>,
}

impl GatewayConfig {
    pub fn new() -> Self {
        GatewayConfig {
            dispatcher : HashMap::new(),
            servers : HashMap::new(),
        }
    } 

    pub async fn load_from_json_value(&mut self, json_value: serde_json::Value) -> Result<(),String> {
        //register inner serveric
        let inner_services = json_value.get("inner_services");
        if inner_services.is_some() {
            let inner_services = inner_services.unwrap();
            let inner_services = inner_services.as_object();
            if inner_services.is_some() {
                for (server_id,server_config) in inner_services.unwrap().iter() {
                    let server_type = server_config.get("type");
                    if server_type.is_none() {
                        return Err("Server type not found".to_string());
                    }
                    let server_type = server_type.unwrap().as_str();
                    if server_type.is_none() {
                        return Err("Server type not string".to_string());
                    }
                    let server_type = server_type.unwrap();
                    match server_type {
                        "cyfs_sn" => {
                            let sn_config = serde_json::from_value::<SNServerConfig>(server_config.clone());
                            if sn_config.is_err() {
                                return Err(format!("Invalid sn config: {}",sn_config.err().unwrap()));
                            }
                            let sn_config = sn_config.unwrap();
                            let sn_server = SNServer::new(Some(sn_config));
                            register_sn_server(server_id, sn_server.clone()).await;
                            info!("Register sn server: {:?}",server_id);
                            register_inner_service_builder(server_id, move || {  
                                Box::new(sn_server.clone())
                            }).await;
                        },
                        _ => {
                            return Err(format!("Invalid server type: {}",server_type));
                        },
                    }
                }
            }
        }

        //register_inner_service_builder("cyfs_sn",|| {
        //    Box::new(SNServer::new(None))
        //}).await;
        
        //load servers
        let servers = json_value.get("servers").unwrap();
        let servers = servers.as_object();
        if servers.is_some() {
            let servers = servers.unwrap();
            for (k,v) in servers.iter() {
                let server_type = v.get("type");
                if server_type.is_none() {
                    return Err("Server type not found".to_string());
                }
                let server_type = server_type.unwrap().as_str();
                if server_type.is_none() {
                    return Err("Server type not string".to_string());
                }
                let server_type = server_type.unwrap();
                match server_type {
                    "cyfs-warp" => {
                        let warp_config = serde_json::from_value::<WarpServerConfig>(v.clone());
                        if warp_config.is_err() {
                            return Err(format!("Invalid warp config: {}",warp_config.err().unwrap()));
                        }
                        let warp_config = warp_config.unwrap();
                        self.servers.insert(k.clone(),ServerConfig::Warp(warp_config));
                    },
                    "cyfs-dns" => {
                        let dns_config = serde_json::from_value::<DNSServerConfig>(v.clone());
                        if dns_config.is_err() {
                            return Err(format!("Invalid dns config: {}",dns_config.err().unwrap()));
                        }
                        let dns_config = dns_config.unwrap();
                        self.servers.insert(k.clone(),ServerConfig::DNS(dns_config));
                    },
                    _ => {
                        return Err(format!("Invalid server type: {}",server_type));
                    },
                }
                                
            }
        }

        //load dispatcher
        let dispatcher = json_value.get("dispatcher").unwrap();
        let dispatcher = dispatcher.as_object().unwrap();
        for (k,v) in dispatcher.iter() {
            let incoming_url = Url::parse(&k);
            if incoming_url.is_err() {
                return Err("Invalid incoming url".to_string());
            }
            let incoming_url = incoming_url.unwrap();
            let incoming_url2 = incoming_url.clone();

            let target_type:Result<String,_> = from_value(v["type"].clone());
            if target_type.is_err() {
                return Err("Target type not found".to_string());
            }
            let target_type = target_type.unwrap();
            let enable_tunnel: Result<Vec<String>, _> = from_value(v["enable_tunnel"].clone());
            let enable_tunnel = enable_tunnel.ok();
            let new_config : DispatcherConfig;
            match target_type.as_str() {
                "server" => {
                    let server_id = v.get("id");
                    if server_id.is_none() {
                        return Err("Server id not found".to_string());
                    }
                    let server_id = server_id.unwrap().as_str();
                    if server_id.is_none() {
                        return Err("Server id not string".to_string());
                    }
                    let server_id = server_id.unwrap();
                    new_config = DispatcherConfig::new_server(incoming_url,server_id.to_string(),enable_tunnel);
                },
                "forward" => {
                    let target_url = v.get("target");
                    if target_url.is_none() {
                        return Err("Target url not found".to_string());
                    }
                    let target_url = target_url.unwrap().as_str();
                    if target_url.is_none() {
                        return Err("Target url not string".to_string());
                    }
                    let target_url = target_url.unwrap();
                    let target_url = Url::parse(target_url);
                    if target_url.is_err() {
                        return Err("Invalid target url".to_string());
                    }
                    let target_url = target_url.unwrap();
                    new_config = DispatcherConfig::new_forward(incoming_url,target_url,enable_tunnel);
                },
                _ => {
                    return Err(format!("Invalid target type: {}",target_type));
                },
            }
            self.dispatcher.insert(incoming_url2,new_config);
        }
        Ok(())
    }
}