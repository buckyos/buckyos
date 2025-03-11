#![allow(dead_code)]

mod provider;
mod name_client;
mod name_query;
mod dns_provider;
mod utility;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::DecodingKey;
pub use provider::*;
pub use name_client::*;
pub use name_query::*;
pub use dns_provider::*;
pub use utility::*;

use cfg_if::cfg_if;
cfg_if! {
    if #[cfg(feature = "cloudflare")] {
        mod cloudflare;
        pub use cloudflare::*;
    }
}


use log::*;
use std::net::IpAddr;
use once_cell::sync::OnceCell;
use name_lib::*;

#[macro_use]
extern crate log;
//TODO 首次初始化的BOOT NAME CLIENT 可以为系统的名字解析提供一个保底
pub static GLOBAL_BOOT_NAME_CLIENT: OnceCell<NameClient> = OnceCell::new();
pub static GLOBAL_NAME_CLIENT: OnceCell<NameClient> = OnceCell::new();



pub async fn resolve_ip(name: &str) -> NSResult<IpAddr> {
    let name_info = resolve(name,None).await?;
    if name_info.address.is_empty() {
        return Err(NSError::NotFound("A record not found".to_string()));
    }
    let result_ip = name_info.address[0];
    Ok(result_ip)
}


pub async fn init_default_name_client() -> NSResult<()> {
    let client = NameClient::new(NameClientConfig::default());
    client.add_provider(Box::new(DnsProvider::new(None))).await;
    let set_result = GLOBAL_NAME_CLIENT.set(client);
    if set_result.is_err() {
        return Err(NSError::Failed("Failed to set GLOBAL_BOOT_NAME_CLIENT".to_string()));
    }
    Ok(())
}


fn get_name_client() -> Option<&'static NameClient> {
    let client = GLOBAL_NAME_CLIENT.get();
    return client;
}

pub async fn resolve(name: &str, record_type: Option<RecordType>) -> NSResult<NameInfo> {
    let client = get_name_client();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not init yet".to_string()));
    }
    let client = client.unwrap();
    client.resolve(name, record_type).await
}

pub async fn resolve_auth_key(hostname: &str) -> NSResult<DecodingKey> {
    //return #auth-key
    //let did = DID::from_host_name(hostname);
    // if did.is_some(){
    //     let did = did.unwrap();
    //     if let Some(auth_key) = did.get_auth_key() {
    //         return Ok((auth_key,hostname.to_string()));
    //     }
    // }

    let client = get_name_client();
    if client.is_none() {
        let msg = "Name client not init yet".to_string();
        error!("{}",msg);
        return Err(NSError::InvalidState(msg));
    }
    let did_doc = client.unwrap().resolve_did(hostname,None).await?;
    //info!("did_doc: {:?}",did_doc);
    // did_doc could be ZoneConfig or DeviceConfig
    let zone_config = ZoneConfig::decode(&did_doc, None);
    if zone_config.is_ok() {
        let zone_config = zone_config.unwrap();
        let auth_key = zone_config.auth_key; 
        if auth_key.is_some() {
            let auth_key = auth_key.unwrap();
            let auth_key = DecodingKey::from_jwk(&auth_key)
                .map_err(|e|NSError::InvalidState(format!("Failed to decode auth key:{}",e.to_string())))?;
            return Ok(auth_key);
        }
    }
    return Err(NSError::NotFound("Invalid did document".to_string()));
}

pub async fn resolve_ed25519_auth_key(hostname: &str) -> NSResult<([u8; 32],String)> {
    //return #auth-key
    let did = DID::from_host_name(hostname);
    if did.is_some(){
        let did = did.unwrap();
        if let Some(auth_key) = did.get_auth_key() {
            return Ok((auth_key,hostname.to_string()));
        }
    }

    let client = get_name_client();
    if client.is_none() {
        let msg = "Name client not init yet".to_string();
        error!("{}",msg);
        return Err(NSError::InvalidState(msg));
    }
    let did_doc = client.unwrap().resolve_did(hostname,None).await?;
    //info!("did_doc: {:?}",did_doc);
    // did_doc could be ZoneConfig or DeviceConfig
    let zone_config = ZoneConfig::decode(&did_doc, None);
    if zone_config.is_ok() {
        let zone_config = zone_config.unwrap();
        if zone_config.device_list.is_some() {
            let device_list = zone_config.device_list.unwrap();
            for device_did in device_list {
                let device_did = DID::from_str(device_did.as_str());
                if device_did.is_some() {
                    let device_did = device_did.unwrap();
                    if let Some(auth_key) = device_did.get_auth_key() {
                        return Ok((auth_key,hostname.to_string()));
                    }
                }
            }
        }
        
        let auth_key = zone_config.auth_key;
        if auth_key.is_some() {
            let auth_key = auth_key.unwrap();
            let auth_key = serde_json::to_value(&auth_key);
            let auth_key = auth_key.unwrap();
            let x = auth_key.get("x");
            if x.is_some() {
                let x = x.unwrap();
                let x = x.as_str().unwrap();
                //let did_id = format!("did:dev:{}",x);
                let auth_key = URL_SAFE_NO_PAD.decode(x).unwrap();
                return Ok((auth_key.try_into().unwrap(),hostname.to_string()));
            }
        }
    }
    return Err(NSError::NotFound("Invalid did document".to_string()));
}


pub async fn resolve_did(did: &str,fragment:Option<&str>) -> NSResult<EncodedDocument> {
    let client = get_name_client();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.resolve_did(did,fragment).await
}

pub async fn add_did_cache(did: &str, doc:EncodedDocument) -> NSResult<()> {
    let client = get_name_client();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.add_did_cache(did, doc)
}

pub async fn add_nameinfo_cache(name: &str, info:NameInfo) -> NSResult<()> {
    let client = get_name_client();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.add_nameinfo_cache(name, info)
}



#[cfg(test)]
mod tests {
    use super::*;


    #[tokio::test]
    async fn test_resolve_did_nameinfo() {
        std::env::set_var("BUCKY_LOG", "debug");
        let service_name = "name-client-test";
        
        buckyos_kit::init_logging(service_name);
        init_default_name_client().await.unwrap();
        let name_info = resolve("test.buckyos.io", crate::provider::RecordType::from_str("DID")).await.unwrap();
        println!("name_info: {:?}",name_info);
    }

}
