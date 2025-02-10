use async_trait::async_trait;
use buckyos_kit::*;
use serde_json::{Value,json};
use std::{net::IpAddr, process::exit};
use std::result::Result;
use ::kRPC::*;
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use name_lib::*;
use name_client::*;
use log::*;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use sys_config::*;

#[derive(Clone)]
struct ActiveServer {
}

impl ActiveServer {
    pub fn new() -> Self {
        ActiveServer {}
    }

    async fn handel_do_active(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let user_name = req.params.get("user_name");
        let zone_name = req.params.get("zone_name");
        let gateway_type = req.params.get("gateway_type");
        let owner_public_key = req.params.get("public_key");
        let owner_private_key = req.params.get("private_key");
        let owner_password_hash = req.params.get("admin_password_hash");
        let enable_guest_access = req.params.get("guest_access");
        let friend_passcode = req.params.get("friend_passcode");
        let device_public_key = req.params.get("device_public_key");
        let device_private_key = req.params.get("device_private_key");
        let support_container = req.params.get("support_container");
        let sn_url = req.params.get("sn_url");
        //let device_info = req.params.get("device_info");  
        if user_name.is_none() || zone_name.is_none() || gateway_type.is_none() || owner_public_key.is_none() || owner_private_key.is_none() || device_public_key.is_none() || device_private_key.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, user_name, zone_name, gateway_type, owner_public_key, owner_private_key, device_public_key or device_private_key is none".to_string()));
        }

        let user_name = user_name.unwrap().as_str().unwrap();
        let zone_name = zone_name.unwrap().as_str().unwrap();
        let gateway_type = gateway_type.unwrap().as_str().unwrap();
        let owner_public_key = owner_public_key.unwrap();
    
        let owner_private_key = owner_private_key.unwrap().as_str().unwrap();
        let device_public_key = device_public_key.unwrap();
        let device_private_key = device_private_key.unwrap().as_str().unwrap();

        let owner_private_key_pem = EncodingKey::from_ed_pem(owner_private_key.as_bytes())
            .map_err(|_|RPCErrors::ReasonError("Invalid owner private key".to_string()))?;
        let device_private_key_pem = EncodingKey::from_ed_pem(device_private_key.as_bytes())
            .map_err(|_|RPCErrors::ReasonError("Invalid device private key".to_string()))?;

        let device_did = get_device_did_from_ed25519_jwk(&device_public_key).unwrap();
        let device_public_jwk = serde_json::from_value(device_public_key.clone()).unwrap();
        let device_ip:Option<IpAddr> = None;
        let mut net_id:Option<String> = None;
        let mut ddns_sn_url:Option<String> = None;
        let mut need_sn = false;
        let mut is_support_container = true;
        if support_container.is_some() {
            is_support_container = support_container.unwrap().as_str().unwrap() == "true";
        }
        //create device doc ,and sign it with owner private key
        match gateway_type {
            "BuckyForward" => {
                net_id = None;
            },
            "PortForward" => {
                if zone_name.ends_with(".web3.buckyos.io") {
                    need_sn = true;
                    ddns_sn_url = Some("http://web3.buckyos.io/kapi/sn".to_string());
                }
                net_id = Some("wan".to_string());
            },
            _ => {
                return Err(RPCErrors::ReasonError("Invalid gateway type".to_string()));
            }
        }

        let device_config:DeviceConfig = DeviceConfig {
            did: device_did.clone(),
            name: "ood1".to_string(),
            arch: None,
            device_type: "ood".to_string(),
            auth_key: device_public_jwk,
            iss: user_name.to_string(),
            ip:None,
            net_id:net_id,
            ddns_sn_url:ddns_sn_url,
            support_container: is_support_container,
            exp: buckyos_get_unix_timestamp() + 3600*24*365*10, 
            iat: buckyos_get_unix_timestamp() as u64,
        };
        let device_doc_jwt = device_config.encode(Some(&owner_private_key_pem))
            .map_err(|_|RPCErrors::ReasonError("Failed to encode device config".to_string()))?;
        
        if sn_url.is_some() {
            //register to sn 
            let sn_url = sn_url.unwrap().as_str().unwrap();
            if !sn_url.is_empty() {
                if sn_url.starts_with("http://") || sn_url.starts_with("https://") {
                    need_sn = true;
                }
            }
        }
        
            
        if need_sn {
            let sn_url = "http://web3.buckyos.io/kapi/sn";
            info!("Register OOD to sn: {}",sn_url);
            let rpc_token = ::kRPC::RPCSessionToken {
                token_type : ::kRPC::RPCSessionTokenType::JWT,
                nonce : None,
                userid : Some(user_name.to_string()),
                appid:Some("active_service".to_string()),
                exp:Some(buckyos_get_unix_timestamp() + 60),
                iss:Some(user_name.to_string()),
                token:None,
            };
            let user_rpc_token = rpc_token.generate_jwt(None,&owner_private_key_pem)
                .map_err(|_| {
                    warn!("Failed to generate user rpc token");
                    RPCErrors::ReasonError("Failed to generate user rpc token".to_string())})?;
            
            let mut device_info = DeviceInfo::from_device_doc(&device_config);
            device_info.auto_fill_by_system_info().await.unwrap();
            let device_info_json = serde_json::to_string(&device_info).unwrap();
            let device_ip = device_info.ip.unwrap().to_string();

            let sn_result = sn_register_device(sn_url, Some(user_rpc_token.to_string()), 
                user_name, "ood1", &device_did.as_str(), &device_ip, device_info_json.as_str()).await;
            if sn_result.is_err() {
                return Err(RPCErrors::ReasonError(format!("Failed to register device to sn: {}",sn_result.err().unwrap())));
            }
        }

        //write device private key 
        let write_dir = get_buckyos_system_etc_dir();
        let device_private_key_file = write_dir.join("node_private_key.pem");
        tokio::fs::write(device_private_key_file,device_private_key.as_bytes()).await.unwrap();
        let owner_public_key_str = owner_public_key.to_string();
        let owner_public_key_str = owner_public_key_str.replace(":", "=");
        //write device idenity
        let device_identity_str = format!(r#"
zone_name = "{}"
owner_public_key={}
owner_name = "did:ens:{}"
device_doc_jwt = "{}"
zone_nonce = "1234567890"
        "#,zone_name,owner_public_key_str,user_name,device_doc_jwt.to_string());

        let device_identity_file = write_dir.join("node_identity.toml");
        tokio::fs::write(device_identity_file,device_identity_str.as_bytes()).await
            .map_err(|_|RPCErrors::ReasonError("Failed to write device identity".to_string()))?;

        //write boot config
        let start_params_str = serde_json::to_string(&req.params).unwrap();
        let start_params_file = write_dir.join("start_config.json");
        tokio::fs::write(start_params_file,start_params_str.as_bytes()).await
            .map_err(|_|RPCErrors::ReasonError("Failed to write start params".to_string()))?;

            
        info!("Write Active files [.device_private_key.pem,.device_identity.toml,start_config.json] success");
        
        tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            exit(0);
        });
        
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code":0
        })),req.seq))
    }

    async fn handle_generate_key_pair(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let (private_key,public_key) = generate_ed25519_key_pair();
        let public_key_str = public_key.to_string();
        return Ok(RPCResponse::new(RPCResult::Success(json!({
            "private_key":private_key,
            "public_key":public_key
        })),req.seq));
    }

    async fn handle_get_device_info(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let mut device_info = DeviceInfo::new("ood1",None);
        device_info.auto_fill_by_system_info().await.unwrap();
        let device_info_json = serde_json::to_value(device_info).unwrap();
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "device_info":device_info_json
        })),req.seq))
    }

    async fn handle_generate_zone_config_jwt(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let zone_config_str = req.params.get("zone_config");
        let private_key = req.params.get("private_key");
        if zone_config_str.is_none() || private_key.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, zone_config or private_key is none".to_string()));
        }
        let zone_config = zone_config_str.unwrap().as_str().unwrap();
        let private_key = private_key.unwrap().as_str().unwrap();

        info!("will sign zone config: {}",zone_config);
        let mut zone_config:ZoneConfig = serde_json::from_str(zone_config)
            .map_err(|e|RPCErrors::ParseRequestError(format!("Invalid zone config: {}",e.to_string())))?;
        let private_key_pem = EncodingKey::from_ed_pem(private_key.as_bytes())
            .map_err(|e|RPCErrors::ParseRequestError(format!("Invalid private key: {}",e.to_string())))?;
        let zone_config_jwt = zone_config.encode(Some(&private_key_pem))
            .map_err(|e|RPCErrors::ParseRequestError(format!("Failed to encode zone config: {}",e.to_string())))?;
        
        return Ok(RPCResponse::new(RPCResult::Success(json!({
            "zone_config_jwt":zone_config_jwt.to_string()
        })),req.seq));
    }
}

#[async_trait]
impl kRPCHandler for ActiveServer {
    async fn handle_rpc_call(&self, req:RPCRequest,ip_from:IpAddr) -> Result<RPCResponse,RPCErrors> {
        match req.method.as_str() {
            "generate_key_pair" => self.handle_generate_key_pair(req).await,
            "get_device_info" => self.handle_get_device_info(req).await,
            "generate_zone_config" => self.handle_generate_zone_config_jwt(req).await,
            "do_active" => self.handel_do_active(req).await,
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}

pub async fn start_node_active_service() {
    let active_server = ActiveServer::new();
    //register active server as inner service
    register_inner_service_builder("active_server", move || {  
        Box::new(active_server.clone())
    }).await;
    //active server config
    let active_server_dir = get_buckyos_system_bin_dir().join("active");
    let active_server_config = json!({
      "tls_port":3143,
      "http_port":3180,
      "hosts": {
        "*": {
          "enable_cors":true,
          "routes": {
            "/": {
              "local_dir": active_server_dir.to_str().unwrap()
            },
            "/kapi/active" : {
                "inner_service":"active_server"
            }
          } 
        }
      }
    });  

    let active_server_config:WarpServerConfig = serde_json::from_value(active_server_config).unwrap();
    //start!
    info!("start node active service...");
    start_cyfs_warp_server(active_server_config).await;
    tokio::signal::ctrl_c().await.unwrap();
}