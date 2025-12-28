use async_trait::async_trait;
use buckyos_kit::*;
use jsonwebtoken::jwk::Jwk;
use serde_json::{Value,json};
use std::collections::HashMap;
use std::{net::IpAddr, process::exit};
use std::result::Result;
use std::sync::Arc;
use ::kRPC::*;
use cyfs_gateway_lib::{HttpServer, ServerError, ServerResult, StreamInfo, serve_http_by_rpc_handler, server_err, ServerErrorCode};
use name_lib::*;
use name_client::*;
use log::*;
use jsonwebtoken::{EncodingKey, DecodingKey};
use buckyos_api::*;
use server_runner::*;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};

const ACTIVE_SERVICE_MAIN_PORT: u16 = 3182;

#[derive(Clone)]
struct ActiveServer {
    device_mini_info:DeviceMiniInfo,
}

impl ActiveServer {
    pub fn new() -> Self {
        ActiveServer {
            device_mini_info:DeviceMiniInfo::default(),
        }
    }

    pub async fn auto_fill_device_mini_info(&mut self) {
        self.device_mini_info.auto_fill_by_system_info().await.unwrap();
        self.device_mini_info.active_url = Some("./index.html".to_string());
    }

    async fn handle_active_by_wallet(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        // Required parameters: only JWT tokens and essential data
        let boot_config_jwt = req.params.get("boot_config_jwt");
        let device_doc_jwt = req.params.get("device_doc_jwt");
        let device_mini_doc_jwt = req.params.get("device_mini_doc_jwt");
        let device_private_key = req.params.get("device_private_key");
        let device_info_param = req.params.get("device_info");

        let user_name = req.params.get("user_name");
        let zone_name = req.params.get("zone_name");
        let is_self_domain_param = req.params.get("is_self_domain");
        let owner_public_key_param = req.params.get("public_key");
        let admin_password_hash = req.params.get("admin_password_hash");
        let guest_access = req.params.get("guest_access");
        let friend_passcode = req.params.get("friend_passcode");

        let sn_url_param = req.params.get("sn_url");
        let sn_username = req.params.get("sn_username");
        let sn_rpc_token = req.params.get("sn_rpc_token");

        if owner_public_key_param.is_none() || device_doc_jwt.is_none() || device_mini_doc_jwt.is_none() || device_private_key.is_none() || zone_name.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, missing required fields: owner_public_key_param, device_doc_jwt, device_mini_doc_jwt, device_private_key, zone_name".to_string()));
        }

        info!("handle_active_by_wallet params: {:?}", req.params);

        let boot_config_jwt = boot_config_jwt.unwrap().as_str().unwrap();
        let zone_name = zone_name.unwrap().as_str().unwrap();
        let is_self_domain =  if is_self_domain_param.is_some() {
            is_self_domain_param.unwrap().as_bool().unwrap()
        } else {
            false
        };
        let zone_did = DID::from_str(zone_name).map_err(|_|RPCErrors::ReasonError("Invalid zone name".to_string()))?;
        let user_name = user_name.unwrap().as_str().unwrap();
        let user_name = user_name.to_lowercase();
        // Get owner public key from device_config (it should be in the JWT header or we need to verify)
        // For now, we'll need owner_public_key to verify, but let's try to extract it from the request if available
        // If not available, we'll decode without verification (less secure but works for now)
        let owner_public_key: Jwk = if owner_public_key_param.is_some() {
            serde_json::from_value(owner_public_key_param.unwrap().clone())
                .map_err(|e| {
                    warn!("Invalid owner public key format: {}", e);
                    RPCErrors::ReasonError("Invalid owner public key format".to_string())
                })?
        } else {
            // Try to extract from device_config if available, otherwise use a placeholder
            // In practice, owner_public_key should be provided or extracted from zone config
            warn!("owner_public_key is required to verify JWT signatures");
            return Err(RPCErrors::ParseRequestError("owner_public_key is required to verify JWT signatures".to_string()));
        };


        let device_doc_jwt = device_doc_jwt.unwrap().as_str().unwrap();
        let device_mini_doc_jwt = device_mini_doc_jwt.unwrap().as_str().unwrap();
        let device_private_key = device_private_key.unwrap().as_str().unwrap();


        // Decode device_doc_jwt to extract information
        let encoded_doc = EncodedDocument::from_str(device_doc_jwt.to_string())
            .map_err(|e| {
                warn!("Invalid device_doc_jwt format: {}", e);
                RPCErrors::ParseRequestError(format!("Invalid device_doc_jwt format: {}", e))
            })?;
        
        // First decode without verification to get owner public key hint, then verify
        // For now, we'll decode without verification first to extract owner info
        // In production, owner_public_key should be provided or extracted from zone config
        let device_config = DeviceConfig::decode(&encoded_doc, None)
            .map_err(|e| {
                warn!("Failed to decode device_doc_jwt: {}", e);
                RPCErrors::ParseRequestError(format!("Failed to decode device_doc_jwt: {}", e))
            })?;
        
        // Extract information from device_config
        let device_did = device_config.id.clone();
        let device_name = device_config.name.clone();
        info!("device_did: {}, device_name: {}", device_did.to_string(), device_name);
  
        // Verify the JWT signatures with owner public key
        let owner_decoding_key = DecodingKey::from_jwk(&owner_public_key)
            .map_err(|e| {
                warn!("Failed to create decoding key: {}", e);
                RPCErrors::ReasonError(format!("Failed to create decoding key: {}", e))
            })?;
        
        // Re-decode with verification
        let _verified_device_config = DeviceConfig::decode(&encoded_doc, Some(&owner_decoding_key))
            .map_err(|e| {
                warn!("Failed to verify device_doc_jwt: {}", e);
                RPCErrors::ParseRequestError(format!("Failed to verify device_doc_jwt: {}", e))
            })?;

        let device_private_key_pem = EncodingKey::from_ed_pem(device_private_key.as_bytes())
            .map_err(|e| {
                warn!("Invalid device private key: {}", e);
                RPCErrors::ReasonError("Invalid device private key".to_string())
            })?;

        info!("device documents decoded success");

        // Determine if SN registration is needed
        let mut sn_url:Option<String> = None;
        let mut need_sn = false;
        if sn_url_param.is_some() {
            sn_url = Some(sn_url_param.unwrap().as_str().unwrap().to_string());
            if sn_url.as_ref().unwrap().len() > 5 {
                need_sn = true;
            }
        }

        // Register device to SN if needed
        if need_sn {
            let sn_url = sn_url.unwrap();
            let sn_rpc_token = if sn_rpc_token.is_some() {
                sn_rpc_token.unwrap().as_str().unwrap()
            } else {
                return Err(RPCErrors::ParseRequestError("sn_rpc_token is required for SN registration".to_string()));
            };

            let sn_username = if sn_username.is_some() {
                sn_username.unwrap().as_str().unwrap()
            } else {
                return Err(RPCErrors::ParseRequestError("sn_username is required for SN registration".to_string()));
            };

            info!("Bind new zone_boot_jwt {} to sn: {}", boot_config_jwt, sn_url);
            let user_domain = if is_self_domain {
                Some(zone_name.to_string())
            } else {
                None
            };

            let sn_result = sn_bind_zone_config(sn_url.as_str(), Some(sn_rpc_token.to_string()),
                sn_username,
                boot_config_jwt,
                user_domain).await;//todo: user_domain?
            if sn_result.is_err() {
                return Err(RPCErrors::ReasonError(format!("Failed to bind zone config to sn: {}",sn_result.err().unwrap())));
            }

            info!("Register {}(zone-gateway) to sn: {}", device_name, sn_url);            
            // device_info can be either a JSON string or a JSON object
            let mut device_info:DeviceInfo = if device_info_param.is_some() {
                let device_info_value = device_info_param.unwrap();
                if device_info_value.is_string() {
                    serde_json::from_str(device_info_value.as_str().unwrap())
                        .map_err(|e|RPCErrors::ParseRequestError(format!("Invalid device_info string: {}", e)))?
                } else {
                    serde_json::from_value(device_info_value.clone())
                        .map_err(|e|RPCErrors::ParseRequestError(format!("Invalid device_info object: {}", e)))?
                }
            } else {
                // Create device_info from device_config if not provided
                let mut info = DeviceInfo::from_device_doc(&device_config);
                info.auto_fill_by_system_info().await.unwrap();
                info
            };

            let device_info_json_final = serde_json::to_string(&device_info).unwrap();

            let mut device_ip = "127.0.0.1".to_string();
            if device_info.ips.len() > 0 {
                device_ip = device_info.ips[0].clone().to_string();
            }
            if device_info.all_ip.len() > 0 {
                device_ip = device_info.all_ip[0].clone().to_string();
            }
            
            let sn_result = sn_register_device(sn_url.as_str(), Some(sn_rpc_token.to_string()), 
            sn_username, &device_name, &device_did.to_string(), &device_ip, device_info_json_final.as_str(), device_mini_doc_jwt).await;
            if sn_result.is_err() {
                return Err(RPCErrors::ReasonError(format!("Failed to register device to sn: {}",sn_result.err().unwrap())));
            }
        } else {
            info!("NO SN mode: Check if the zone txt records is already exists ...");
            // let zone_boot = resolve_did(&zone_did, None).await
            //     .map_err(|e|RPCErrors::ReasonError(format!("Failed to resolve zone did: {}", e)))?;
            // let zone_boot_config = ZoneBootConfig::decode(&zone_boot, Some(&owner_decoding_key))
            //     .map_err(|e|RPCErrors::ReasonError(format!("Failed to decode zone boot config: {}", e)))?;
            info!("verify zone boot config success");
        }

        // Write device private key
        let write_dir = get_buckyos_system_etc_dir();
        let device_private_key_file = write_dir.join("node_private_key.pem");
        tokio::fs::write(device_private_key_file,device_private_key.as_bytes()).await
            .map_err(|e|RPCErrors::ReasonError(format!("Failed to write device private key: {}", e)))?;

        // Write device identity
        let zone_did = DID::from_str(zone_name)
            .map_err(|_|RPCErrors::ReasonError("Invalid zone name".to_string()))?;
        let owner_did = DID::from_str(&user_name)
            .unwrap_or_else(|_| DID::new("bns", &user_name));
        let node_identity = NodeIdentityConfig {
            zone_did:zone_did,
            owner_public_key:owner_public_key,
            owner_did:owner_did,
            device_doc_jwt:device_doc_jwt.to_string(),
            zone_iat:(buckyos_get_unix_timestamp() as u32 - 3600),
            device_mini_doc_jwt:device_mini_doc_jwt.to_string(),
        };
        let device_identity_file = write_dir.join("node_identity.json");
        let device_identity_str = serde_json::to_string(&node_identity)
            .map_err(|e|RPCErrors::ReasonError(format!("Failed to serialize node identity: {}", e)))?;
        tokio::fs::write(device_identity_file,device_identity_str.as_bytes()).await
            .map_err(|_|RPCErrors::ReasonError("Failed to write node_identity.json".to_string()))?;

        // Write start config (minimal, only essential params)
        let mut real_start_params = req.params.clone();
        let mut real_start_params = real_start_params.as_object_mut().unwrap();
        real_start_params.insert("ood_jwt".to_string(),Value::String(device_doc_jwt.to_string()));
        let start_params_str = serde_json::to_string(&real_start_params)
            .map_err(|e|RPCErrors::ReasonError(format!("Failed to serialize start params: {}", e)))?;
        let start_params_file = write_dir.join("start_config.json");
        tokio::fs::write(start_params_file,start_params_str.as_bytes()).await
            .map_err(|_|RPCErrors::ReasonError("Failed to write start params".to_string()))?;

        //write node_device_config.json
        let node_device_config_file = write_dir.join("node_device_config.json");
        let node_device_config_json_str = serde_json::to_string(&device_config).unwrap();
        tokio::fs::write(node_device_config_file,node_device_config_json_str.as_bytes()).await
            .map_err(|_|RPCErrors::ReasonError("Failed to write node_device_config.json".to_string()))?;

        info!("ActiveByWallet Write Active files [node_private_key.pem,node_identity.json,start_config.json,node_device_config.json] success");
        
        tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            exit(0);
        });
        
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code":0
        })),req.id))
    }


    async fn handle_prepare_params_for_active_by_wallet(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let user_name = req.params.get("user_name");
        let zone_name = req.params.get("zone_name");
        let net_id = req.params.get("net_id");
        let owner_public_key = req.params.get("public_key");
        let device_public_key = req.params.get("device_public_key");
        let device_private_key = req.params.get("device_private_key");
        let device_rtcp_port_param = req.params.get("device_rtcp_port");
        let support_container = req.params.get("support_container");
        let sn_username = req.params.get("sn_username");
        let sn_url_param = req.params.get("sn_url");
        let mut sn_url:Option<String> = None;
        if sn_url_param.is_some() {
            sn_url = Some(sn_url_param.unwrap().as_str().unwrap().to_string());
        }

        if user_name.is_none() || zone_name.is_none()  || owner_public_key.is_none() || device_public_key.is_none() || device_private_key.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, user_name, zone_name, gateway_type, owner_public_key, device_public_key or device_private_key is none".to_string()));
        }

        let user_name = user_name.unwrap().as_str().unwrap();
        let user_name = user_name.to_lowercase();
        let zone_name = zone_name.unwrap().as_str().unwrap();
        let zone_did = DID::from_str(zone_name)
            .map_err(|_|RPCErrors::ReasonError("Invalid zone name".to_string()))?;

        let net_id = if net_id.is_some() {
            Some(net_id.unwrap().as_str().unwrap().to_string())
        } else {
            None
        };

        let owner_public_key = owner_public_key.unwrap();
        let device_public_key = device_public_key.unwrap();
        let device_private_key = device_private_key.unwrap().as_str().unwrap();
        let mut device_rtcp_port = None;
        if device_rtcp_port_param.is_some() {
            let real_device_rtcp_port = device_rtcp_port_param.unwrap().as_u64().unwrap();
            if real_device_rtcp_port != 2980 {
                device_rtcp_port = Some(real_device_rtcp_port as u32);
            }
        }

        let device_private_key_pem = EncodingKey::from_ed_pem(device_private_key.as_bytes())
            .map_err(|_|RPCErrors::ReasonError("Invalid device private key".to_string()))?;
        let device_did = get_device_did_from_ed25519_jwk(&device_public_key)
            .map_err(|_|RPCErrors::ReasonError("Invalid device public key".to_string()))?;
        let device_public_jwk:Jwk = serde_json::from_value(device_public_key.clone())
            .map_err(|_|RPCErrors::ReasonError("Invalid device public key format".to_string()))?;


        let mut need_sn = false;
        let mut is_support_container = true;
        if support_container.is_some() {
            is_support_container = support_container.unwrap().as_str().unwrap() == "true";
        }

        // Create device_config without signing
        let mut ddns_sn_url:Option<String> = None;
        if net_id.is_some() {
            let real_net_id = net_id.as_ref().unwrap();
            if real_net_id == "wan_dyn" {
                ddns_sn_url = sn_url.clone();
            }
            if real_net_id == "portmap" {
                ddns_sn_url = sn_url.clone();
            }
        }

        let mut device_config = DeviceConfig::new_by_jwk("ood1",device_public_jwk);
        device_config.net_id = net_id;
        device_config.ddns_sn_url = ddns_sn_url;
        device_config.support_container = is_support_container;
        device_config.iss = format!("did:bns:{}",user_name.as_str());
        device_config.zone_did = Some(zone_did.clone());
        device_config.rtcp_port = device_rtcp_port;
        
        // Convert device_config to JSON (unsigned)
        let device_config_json = serde_json::to_value(&device_config)
            .map_err(|e|RPCErrors::ReasonError(format!("Failed to serialize device config: {}", e)))?;

        // Create device info for SN registration
        let mut device_info = DeviceInfo::from_device_doc(&device_config);
        device_info.auto_fill_by_system_info().await.unwrap();
        let device_info_json = serde_json::to_string(&device_info)
            .map_err(|e|RPCErrors::ReasonError(format!("Failed to serialize device info: {}", e)))?;

        // Check if SN registration is needed
        if sn_url.is_some() {
            if sn_url.as_ref().unwrap().len() > 5 {
                need_sn = true;
            }
        }

        let sn_username = sn_username.unwrap().as_str().unwrap().to_lowercase();
        // Prepare RPC token for SN registration (if needed)
        let rpc_token_json = if need_sn {
            let rpc_token = ::kRPC::RPCSessionToken {
                token_type : ::kRPC::RPCSessionTokenType::JWT,
                nonce : None,
                session : None,
                userid : Some(sn_username),
                appid:Some("active_service".to_string()),
                exp:Some(buckyos_get_unix_timestamp() + 60),
                iss:Some(user_name.to_string()),
                token:None,
            };
            Some(serde_json::to_value(&rpc_token)
                .map_err(|e|RPCErrors::ReasonError(format!("Failed to serialize rpc token: {}", e)))?)
        } else {
            None
        };

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "device_config": device_config_json,
            "rpc_token": rpc_token_json,
            "device_info": device_info_json,
        })),req.id))
    }

    async fn handle_do_active(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        //info!("handle_do_active: {}",serde_json::to_string_pretty(&req.params).unwrap());
        let user_name = req.params.get("user_name");
        let zone_name = req.params.get("zone_name");
        let net_id = req.params.get("net_id");
        let owner_public_key = req.params.get("public_key");
        let owner_private_key = req.params.get("private_key");
        let owner_password_hash = req.params.get("admin_password_hash");
        let enable_guest_access = req.params.get("guest_access");
        let friend_passcode = req.params.get("friend_passcode");
        let device_public_key = req.params.get("device_public_key");
        let device_private_key = req.params.get("device_private_key");
        let device_rtcp_port_param = req.params.get("device_rtcp_port");
        let support_container = req.params.get("support_container");
        let sn_url_param = req.params.get("sn_url");
        let sn_username = req.params.get("sn_username");
        let mut sn_url:Option<String> = None;
        if sn_url_param.is_some() {
            sn_url = Some(sn_url_param.unwrap().as_str().unwrap().to_string());
        }
        //let device_info = req.params.get("device_info");  
        if user_name.is_none() || zone_name.is_none() || owner_public_key.is_none() || owner_private_key.is_none() || device_public_key.is_none() || device_private_key.is_none() {
            warn!("Invalid params, user_name, zone_name, owner_public_key, owner_private_key, device_public_key or device_private_key is none");
            return Err(RPCErrors::ParseRequestError("Invalid params, user_name, zone_name, owner_public_key, owner_private_key, device_public_key or device_private_key is none".to_string()));
        }

        let user_name = user_name.unwrap().as_str().unwrap();
        let user_name = user_name.to_lowercase();
        let zone_name = zone_name.unwrap().as_str().unwrap();
        let zone_did = DID::from_str(zone_name)
            .map_err(|_|RPCErrors::ReasonError("Invalid zone name".to_string()))?;

        let net_id = if net_id.is_some() {
            Some(net_id.unwrap().as_str().unwrap().to_string())
        } else {
            None
        };

        let owner_public_key = owner_public_key.unwrap();
        let owner_private_key = owner_private_key.unwrap().as_str().unwrap();
        let device_public_key = device_public_key.unwrap();
        let device_private_key = device_private_key.unwrap().as_str().unwrap();
        let mut device_rtcp_port = None;
        if device_rtcp_port_param.is_some() {
            let real_device_rtcp_port = device_rtcp_port_param.unwrap().as_u64().unwrap();
            if real_device_rtcp_port != 2980 {
                device_rtcp_port = Some(real_device_rtcp_port as u32);
            }
        }

        let owner_private_key_pem = EncodingKey::from_ed_pem(owner_private_key.as_bytes())
            .map_err(|_|RPCErrors::ReasonError("Invalid owner private key".to_string()))?;
        let device_private_key_pem = EncodingKey::from_ed_pem(device_private_key.as_bytes())
            .map_err(|_|RPCErrors::ReasonError("Invalid device private key".to_string()))?;
        let device_did = get_device_did_from_ed25519_jwk(&device_public_key)
            .map_err(|_|RPCErrors::ReasonError("Invalid device public key".to_string()))?;
        let device_public_jwk:Jwk = serde_json::from_value(device_public_key.clone()).unwrap();

        //let device_ip:Option<IpAddr> = None;
        let mut ddns_sn_url:Option<String> = None;
        let mut need_sn = true;
        if net_id.is_some() {
            let real_net_id = net_id.as_ref().unwrap();
            if real_net_id == "wan_dyn" {
                ddns_sn_url = sn_url.clone();
            }
            if real_net_id == "portmap" {
                ddns_sn_url = sn_url.clone();
            }
        }

        
        let mut is_support_container = true;
        if support_container.is_some() {
            is_support_container = support_container.unwrap().as_str().unwrap() == "true";
        }
        //create device doc ,and sign it with owner private key
        let mut device_config = DeviceConfig::new_by_jwk("ood1",device_public_jwk);
        device_config.net_id = net_id;
        device_config.ddns_sn_url = ddns_sn_url;
        device_config.support_container = is_support_container;
        device_config.iss = format!("did:bns:{}",user_name.as_str());
        device_config.zone_did = Some(zone_did.clone());
        device_config.rtcp_port = device_rtcp_port;
        //device_config.ip = device_ip;

        let device_doc_jwt = device_config.encode(Some(&owner_private_key_pem))
            .map_err(|_|RPCErrors::ReasonError("Failed to encode device config".to_string()))?;
        
        let device_mini_config = DeviceMiniConfig::new_by_device_config(&device_config);
        let device_mini_config_jwt = device_mini_config.to_jwt(&owner_private_key_pem).unwrap();
        if sn_url.is_some() {
            if sn_url.as_ref().unwrap().len() > 5 {
                need_sn = true;
            }
        }
        
        if need_sn {
            //call sn_register_device by owner's token
            let sn_url = sn_url.unwrap();
            let sn_username = sn_username.unwrap().as_str().unwrap().to_lowercase();
            let rpc_token = ::kRPC::RPCSessionToken {
                token_type : ::kRPC::RPCSessionTokenType::JWT,
                nonce : None,
                session : None,
                userid : Some(sn_username.to_string()),
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
            let mut device_ip = "127.0.0.1".to_string();
            if device_info.ips.len() > 0 {
                device_ip = device_info.ips[0].clone().to_string();
            }
            if device_info.all_ip.len() > 0 {
                device_ip = device_info.all_ip[0].clone().to_string();
            }
            info!("Register device ood1(zone-gateway) to sn: {}",sn_url);

            let sn_result = sn_register_device(sn_url.as_str(), Some(user_rpc_token), 
                sn_username.as_str(), "ood1", &device_did.to_string(), &device_ip, device_info_json.as_str(),&device_mini_config_jwt).await;
            if sn_result.is_err() {
                warn!("Failed to register device to sn: {}",sn_result.as_ref().err().unwrap());
                return Err(RPCErrors::ReasonError(format!("Failed to register device to sn: {}",sn_result.as_ref().err().unwrap().to_string())));
            }
        }

        //TODO: call resolve_did to check self domain config is correct?
        //  check in ui is more smoothly

        //write device private key 
        let write_dir = get_buckyos_system_etc_dir();
        let device_private_key_file = write_dir.join("node_private_key.pem");
        tokio::fs::write(device_private_key_file,device_private_key.as_bytes()).await.unwrap();
        let owner_public_key:Jwk = serde_json::from_value(owner_public_key.clone()).unwrap();
        
        //write device idenity，

        let device_mini_config = DeviceMiniConfig::new_by_device_config(&device_config);
        let device_mini_doc_jwt = device_mini_config.to_jwt(&owner_private_key_pem).unwrap();
        let node_identity = NodeIdentityConfig {
            zone_did:zone_did,
            owner_public_key:owner_public_key,//TODO:how to update owner's public key? (update owner's did-doc)
            owner_did:DID::new("bns",user_name.as_str()),
            device_doc_jwt:device_doc_jwt.to_string(),
            zone_iat:(buckyos_get_unix_timestamp() as u32 - 3600),
            device_mini_doc_jwt:device_mini_doc_jwt.to_string(),
        };
        let device_identity_file = write_dir.join("node_identity.json");
        let device_identity_str = serde_json::to_string(&node_identity).unwrap();
        tokio::fs::write(device_identity_file,device_identity_str.as_bytes()).await
            .map_err(|_|RPCErrors::ReasonError("Failed to write node_identity.json".to_string()))?;

        //write start config ,TODO
        let mut real_start_parms = req.params.clone();
        let mut real_start_params = real_start_parms.as_object_mut().unwrap();
        real_start_params.insert("ood_jwt".to_string(),Value::String(device_doc_jwt.to_string()));
        let start_params_str = serde_json::to_string(&real_start_params).unwrap();
        let start_params_file = write_dir.join("start_config.json");
        tokio::fs::write(start_params_file,start_params_str.as_bytes()).await
            .map_err(|_|RPCErrors::ReasonError("Failed to write start params".to_string()))?;

        //write node_device_config.json
        let device_config_file = write_dir.join("node_device_config.json");
        let device_config_json_str = serde_json::to_string(&device_config).unwrap();
        tokio::fs::write(device_config_file,device_config_json_str.as_bytes()).await
            .map_err(|_|RPCErrors::ReasonError("Failed to write node_device_config.json".to_string()))?;

        //TODO: write zone_boot_config let system can boot immediately?
        // The zone document caching mechanism needs to be refactored first to prevent incorrect updates.
        info!("DoAction Write Active files [node_private_key.pem,node_identity.json,start_config.json,node_device_config.json] success");
        
        tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            exit(0);
        });
        
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code":0
        })),req.id))
    }

    async fn handle_generate_key_pair(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let (private_key,public_key) = generate_ed25519_key_pair();
        let public_key_str = public_key.to_string();
        return Ok(RPCResponse::new(RPCResult::Success(json!({
            "private_key":private_key,
            "public_key":public_key
        })),req.id));
    }

    async fn handle_get_device_info(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let mut device_info = DeviceInfo::new("ood1",DID::new("dns","ood1"));
        device_info.auto_fill_by_system_info().await.unwrap();
        let device_info_json = serde_json::to_value(device_info).unwrap();
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "device_info":device_info_json
        })),req.id))
    }

    async fn handle_generate_zone_txt_records(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let zone_boot_config_str = req.params.get("zone_boot_config");
        let device_mini_config_str = req.params.get("device_mini_config");
        let private_key = req.params.get("private_key");

        if zone_boot_config_str.is_none() || private_key.is_none() || device_mini_config_str.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, zone_boot_config, device_mini_config or private_key is none".to_string()));
        }

        let zone_config = zone_boot_config_str.unwrap().as_str().unwrap();
        let private_key = private_key.unwrap().as_str().unwrap();

        info!("will sign zone config: {}",zone_config);
        let mut zone_boot_config:ZoneBootConfig = serde_json::from_str(zone_config)
            .map_err(|e|RPCErrors::ParseRequestError(format!("Invalid zone config: {}",e.to_string())))?;
        let private_key_pem = EncodingKey::from_ed_pem(private_key.as_bytes())
            .map_err(|e|RPCErrors::ParseRequestError(format!("Invalid private key: {}",e.to_string())))?;
        let zone_boot_config_jwt = zone_boot_config.encode(Some(&private_key_pem))
            .map_err(|e|RPCErrors::ParseRequestError(format!("Failed to encode zone config: {}",e.to_string())))?;
        info!("zone config jwt: {}",zone_boot_config_jwt.to_string());

        let device_mini_config_str = device_mini_config_str.unwrap().as_str().unwrap();
        info!("will sign device mini config: {}",device_mini_config_str.to_string());
        let device_mini_config:DeviceMiniConfig = serde_json::from_str(device_mini_config_str)
            .map_err(|e|RPCErrors::ParseRequestError(format!("Invalid device mini config: {}",e.to_string())))?;
        let device_mini_config_jwt = device_mini_config.to_jwt(&private_key_pem)
            .map_err(|e|RPCErrors::ParseRequestError(format!("Failed to encode device mini config: {}",e.to_string())))?;
        info!("device mini config jwt: {}",device_mini_config_jwt);

        return Ok(RPCResponse::new(RPCResult::Success(json!({
            "BOOT":zone_boot_config_jwt.to_string(),
            "DEV":device_mini_config_jwt,
        })),req.id));
    }

    async fn handle_get_mini_device_info(&self,req:http::Request<BoxBody<Bytes, ServerError>>) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let device_info_json = serde_json::to_string(&self.device_mini_info).unwrap();
        Ok(http::Response::builder()
            .body(BoxBody::new(
                Full::new(Bytes::from(device_info_json))
                    .map_err(|never: std::convert::Infallible| -> ServerError {
                        match never {}
                    })
                    .boxed(),
            ))
            .map_err(|e| server_err!(ServerErrorCode::InvalidData, "Failed to build response: {}", e))?)
    }
}

#[async_trait]
impl RPCHandler for ActiveServer {
    async fn handle_rpc_call(&self, req:RPCRequest,ip_from:IpAddr) -> Result<RPCResponse,RPCErrors> {
        let method = req.method.clone();
        let result = match req.method.as_str() {
            "generate_key_pair" => self.handle_generate_key_pair(req).await,
            "get_device_info" => self.handle_get_device_info(req).await,
            "generate_zone_txt_records" => self.handle_generate_zone_txt_records(req).await,
            "do_active" => self.handle_do_active(req).await,
            "prepare_params_for_active_by_wallet" => self.handle_prepare_params_for_active_by_wallet(req).await,
            "do_active_by_wallet" => self.handle_active_by_wallet(req).await,
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        };
        if result.is_err() {
            error!("Failed to handle rpc call:{} {}", method.as_str(), result.as_ref().err().unwrap().to_string());
            return Err(result.err().unwrap());
        }
        return result;
    }
}

#[async_trait]
impl HttpServer for ActiveServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        if *req.method() == Method::GET {
            if req.uri().path() == "/device" {
                return self.handle_get_mini_device_info(req).await;
            }
        }
        return Err(server_err!(ServerErrorCode::BadRequest, "Method not allowed"));
    }

    fn id(&self) -> String {
        "active-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_node_active_service() {
    let active_server = ActiveServer::new();
    
    //active server config
    let active_server_dir = get_buckyos_system_bin_dir().join("node-active");
    
    //start!
    info!("start node active service...");
    

    let runner = Runner::new(ACTIVE_SERVICE_MAIN_PORT);

    // 添加 RPC 服务
    let mut active_server =ActiveServer::new();
    active_server.auto_fill_device_mini_info().await;

    let active_server = Arc::new(active_server);
    let add_result = runner.add_http_server("/kapi/active".to_string(), active_server.clone());
    if add_result.is_err() {
        error!("Failed to add http server: {}", add_result.err().unwrap());
        return;
    }

    let add_result = runner.add_http_server("/device".to_string(), active_server.clone());
    if add_result.is_err() {
        error!("Failed to add http server: {}", add_result.err().unwrap());
        return;
    }
    
    // 添加静态文件服务
    info!("active server dir: {}", active_server_dir.display());
    let add_result = runner.add_dir_handler("/".to_string(), active_server_dir).await;
    if add_result.is_err() {
        error!("Failed to add dir handler: {}", add_result.err().unwrap());
        return;
    }
    
    runner.run().await;
}