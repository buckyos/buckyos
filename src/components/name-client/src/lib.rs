#![allow(dead_code)]

mod provider;
mod name_client;
mod name_query;
mod dns_provider;
mod utility;

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
use std::collections::HashMap;
use std::net::IpAddr;
use once_cell::sync::OnceCell;
use name_lib::*;

#[macro_use]
extern crate log;
//TODO 首次初始化的BOOT NAME CLIENT 可以为系统的名字解析提供一个保底
pub static GLOBAL_BOOT_NAME_CLIENT: OnceCell<NameClient> = OnceCell::new();
pub static GLOBAL_NAME_CLIENT: OnceCell<NameClient> = OnceCell::new();


pub fn get_default_web3_bridge_config() -> HashMap<String, String> {
    let mut web3_bridge_config = HashMap::new();
    web3_bridge_config.insert("bns".to_string(), "web3.buckyos.org".to_string());
    web3_bridge_config
}

//name lib 是系统最基础的库，应尽量在进程启动时完成初始化
pub async fn init_name_lib(web3_bridge_config:&HashMap<String, String>) -> NSResult<()> {
    //init web3 bridge config


    let set_result = KNOWN_WEB3_BRIDGE_CONFIG.set(web3_bridge_config.clone());
    if set_result.is_err() {
        return Err(NSError::Failed("Failed to set KNOWN_WEB3_BRIDGE_CONFIG".to_string()));
    }

    let client = NameClient::new(NameClientConfig::default());
    client.add_provider(Box::new(DnsProvider::new(None))).await;
    let set_result = GLOBAL_NAME_CLIENT.set(client);
    if set_result.is_err() {
        return Err(NSError::Failed("Failed to set GLOBAL_BOOT_NAME_CLIENT".to_string()));
    }
    
    Ok(())
}


pub async fn resolve_ip(name: &str) -> NSResult<IpAddr> {
    let name_info = resolve(name,None).await?;
    if name_info.address.is_empty() {
        return Err(NSError::NotFound("A record not found".to_string()));
    }
    let result_ip = name_info.address[0];
    Ok(result_ip)
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

pub async fn resolve_auth_key(did: &DID,kid:Option<&str>) -> NSResult<DecodingKey> {
    let ed25519_auth_key = did.get_ed25519_auth_key();
    if ed25519_auth_key.is_some() {
        let auth_key = ed25519_to_decoding_key(&ed25519_auth_key.unwrap())?;
        return Ok(auth_key);
    }

    let client = get_name_client();
    if client.is_none() {
        let msg = "Name client not init yet".to_string();
        error!("{}",msg);
        return Err(NSError::InvalidState(msg));
    }
    let did_doc = client.unwrap().resolve_did(did,None).await?;
    let did_doc = parse_did_doc(did_doc)?;
    let auth_key = did_doc.get_auth_key(kid);
    if auth_key.is_some() {
        return Ok(auth_key.unwrap());
    }
    return Err(NSError::NotFound("Invalid kid".to_string()));
}

pub async fn resolve_ed25519_auth_key(remote_did: &DID) -> NSResult<[u8; 32]> {
    //return #auth-key
    if let Some(auth_key) = remote_did.get_ed25519_auth_key() {
        return Ok(auth_key);
    }
    
    let client = get_name_client();
    if client.is_none() {
        let msg = "Name client not init yet".to_string();
        error!("{}",msg);
        return Err(NSError::InvalidState(msg));
    }
    let did_doc = client.unwrap().resolve_did(remote_did,None).await?;
    let did_doc = parse_did_doc(did_doc)?;
    let exchange_key = did_doc.get_exchange_key(None);
    if exchange_key.is_some() {
        let exchange_key = exchange_key.unwrap();
        let exchange_key = decoding_key_to_ed25519_sk(&exchange_key)?;
        return Ok(exchange_key);
    }
    return Err(NSError::NotFound("Invalid did document".to_string()));
}


pub async fn resolve_did(did: &DID ,fragment:Option<&str>) -> NSResult<EncodedDocument> {
    let client = get_name_client();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.resolve_did(did,fragment).await
}

pub async fn add_did_cache(did: DID, doc:EncodedDocument) -> NSResult<()> {
    let client = get_name_client();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.add_did_cache(did, doc)
}

pub async fn add_nameinfo_cache(hostname: &str, info:NameInfo) -> NSResult<()> {
    let client = get_name_client();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.add_nameinfo_cache(hostname, info)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[tokio::test]
    async fn test_resolve_did_nameinfo() {
        std::env::set_var("BUCKY_LOG", "debug");
        let service_name = "name-client-test";
        let web3_bridge_config = get_default_web3_bridge_config();
        buckyos_kit::init_logging(service_name,false);
        init_name_lib(&web3_bridge_config).await.unwrap();
        let name_info = resolve("test.buckyos.io", crate::provider::RecordType::from_str("DID")).await.unwrap();
        println!("name_info: {:?}",name_info);
    }

}
