use std::collections::HashMap;
use std::path::PathBuf;
use cyfs_gateway_lib::DNSServerConfig;
use cyfs_gateway_lib::DispatcherConfig;
use cyfs_gateway_lib::ServerConfig;
use cyfs_gateway_lib::WarpServerConfig;
use cyfs_gateway_lib::CURRENT_DEVICE_RRIVATE_KEY;
use cyfs_sn::*;
use cyfs_warp::register_inner_service_builder;
use url::Url;
use serde_json::from_value;
use log::*;
use buckyos_kit::{adjust_path, get_buckyos_root_dir};
use name_lib::load_pem_private_key;

pub struct GatewayConfig {
    pub dispatcher : HashMap<Url,DispatcherConfig>,
    pub servers : HashMap<String,ServerConfig>,
    pub device_key_path: PathBuf,
    //tunnel_builder_config : HashMap<String,TunnelBuilderConfig>,
}

impl GatewayConfig {

    pub async fn load_from_json_value(json_value: serde_json::Value) -> Result<Self,String> {
        let mut device_key_path = PathBuf::new();
        if let Some(Some(path)) = json_value.get("device_key_path").map(|p| p.as_str()) {
            device_key_path = adjust_path(path).map_err(|e| format!("adjust path failed! {}",e))?;
            info!("adjust device key path {} to {}", path, device_key_path.display());
            let private_key_array = load_pem_private_key(&device_key_path)
                .map_err(|e| format!("load device private key failed! {}",e))?;
            CURRENT_DEVICE_RRIVATE_KEY.set(private_key_array).unwrap();
            info!("load device private key success!");
        }
        
        //register inner serveric
        if let Some(Some(inner_services)) = json_value.get("inner_services").map(|v|v.as_object()) {
            for (server_id,server_config) in inner_services.iter() {
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

        //register_inner_service_builder("cyfs_sn",|| {
        //    Box::new(SNServer::new(None))
        //}).await;
        
        //load servers
        let mut servers_cfg = HashMap::new();
        if let Some(Some(servers)) = json_value.get("servers").map(|v| v.as_object()) {
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
                        let mut warp_config = warp_config.unwrap();
                        // adjust warp config`s route local dir path
                        for (host, host_config) in warp_config.hosts.iter_mut() {
                            for (route, route_config) in host_config.routes.iter_mut() {
                               if route_config.local_dir.is_some() {
                                   let new_path = adjust_path(route_config.local_dir.as_ref().unwrap())
                                       .map_err(|e| format!("adjust path failed! {}",e))?;
                                   info!("adjust host {}.{} local path {} to {}", host, route, route_config.local_dir.as_ref().unwrap(), new_path.display());
                                   route_config.local_dir = Some(new_path.to_string_lossy().to_string());
                               }
                            }
                        }
                        servers_cfg.insert(k.clone(),ServerConfig::Warp(warp_config));
                    },
                    "cyfs-dns" => {
                        let dns_config = serde_json::from_value::<DNSServerConfig>(v.clone());
                        if dns_config.is_err() {
                            return Err(format!("Invalid dns config: {}",dns_config.err().unwrap()));
                        }
                        let dns_config = dns_config.unwrap();
                        servers_cfg.insert(k.clone(),ServerConfig::DNS(dns_config));
                    },
                    _ => {
                        return Err(format!("Invalid server type: {}",server_type));
                    },
                }

            }
        }

        //load dispatcher
        let mut dispatcher_cfg = HashMap::new();
        if let Some(Some(dispatcher)) = json_value.get("dispatcher").map(|v| v.as_object()) {
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
                dispatcher_cfg.insert(incoming_url2,new_config);
            }
        }

        Ok(Self {
            dispatcher: dispatcher_cfg,
            servers: servers_cfg,
            device_key_path,
        })
    }
}