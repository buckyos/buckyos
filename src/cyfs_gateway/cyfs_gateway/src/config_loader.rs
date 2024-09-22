
use std::collections::HashMap;
use cyfs_gateway_lib::DispatcherConfig;
use url::Url;
use serde_json::{Value, from_value};
pub struct ConfigLoader {
    pub dispatcher : HashMap<Url,DispatcherConfig>,
    //servers_config : HashMap<String,ServerConfig>,
    //tunnel_builder_config : HashMap<String,TunnelBuilderConfig>,
}

impl ConfigLoader {
    pub fn new() -> Self {
        ConfigLoader {
            dispatcher : HashMap::new(),
            //servers_config : HashMap::new(),
        }
    } 

    pub fn load_from_json_value(&mut self, json_value: serde_json::Value) -> Result<(),String> {
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