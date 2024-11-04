mod provider;
mod name_client;
mod name_query;
mod dns_provider;
mod zone_provider;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, engine::general_purpose::STANDARD,Engine as _};
use jsonwebtoken::{jwk::Jwk, DecodingKey};
pub use provider::*;
pub use name_client::*;
pub use name_query::*;
pub use dns_provider::*;
pub use zone_provider::*;

use log::*;
use std::{env, net::IpAddr};
use once_cell::sync::OnceCell;
use name_lib::*;


pub static GLOBAL_BOOT_NAME_CLIENT: OnceCell<NameClient> = OnceCell::new();
pub static GLOBAL_NAME_CLIENT: OnceCell<NameClient> = OnceCell::new();

pub static CURRENT_APP_SESSION_TOKEN: OnceCell<String> = OnceCell::new();

pub async fn resolve_ip(name: &str) -> NSResult<IpAddr> {
    let name_info = resolve(name,None).await?;
    if name_info.address.is_empty() {
        return Err(NSError::NotFound("A record not found".to_string()));
    }
    let result_ip = name_info.address[0];
    Ok(result_ip)
}

pub fn init_global_buckyos_value_by_env(app_name: &str) {
    let zone_config_str = env::var("BUCKY_ZONE_CONFIG");
    if zone_config_str.is_err() {
        warn!("BUCKY_ZONE_CONFIG not set");
        return;
    }
    let zone_config_str = zone_config_str.unwrap();
    info!("zone_config_str:{}",zone_config_str);    
    let zone_config = serde_json::from_str(zone_config_str.as_str());
    if zone_config.is_err() {
        warn!("zone_config_str format error");
        return;
    }
    let zone_config = zone_config.unwrap();
    let set_result = CURRENT_ZONE_CONFIG.set(zone_config);
    if set_result.is_err() {
        warn!("Failed to set GLOBAL_ZONE_CONFIG");
        return;
    }

    let device_doc = env::var("BUCKY_THIS_DEVICE");
    if device_doc.is_err() {
        warn!("BUCKY_DEVICE_DOC not set");
        return;
    }
    let device_doc = device_doc.unwrap();
    info!("device_doc:{}",device_doc);
    let device_config= serde_json::from_str(device_doc.as_str());
    if device_config.is_err() {
        warn!("device_doc format error");
        return;
    }
    let device_config:DeviceConfig = device_config.unwrap();
    let set_result = CURRENT_DEVICE_CONFIG.set(device_config);
    if set_result.is_err() {
        warn!("Failed to set CURRENT_DEVICE_CONFIG");
        return;
    }

    let session_token_key = format!("{}_SESSION_TOKEN",app_name);
    let session_token = env::var(session_token_key.as_str());
    if session_token.is_err() {
        warn!("{} not set",session_token_key);
        return;
    }
    let session_token = session_token.unwrap();
    let set_result = CURRENT_APP_SESSION_TOKEN.set(session_token);
    if set_result.is_err() {
        warn!("Failed to set GLOBAL_APP_SESSION_TOKEN");
        return;
    }

    
    
}

pub async fn init_default_name_client() -> NSResult<()> {
    let client = NameClient::new(NameClientConfig::default());
    let set_result = GLOBAL_BOOT_NAME_CLIENT.set(client);
    if set_result.is_err() {
        return Err(NSError::Failed("Failed to set GLOBAL_BOOT_NAME_CLIENT".to_string()));
    }
    Ok(())
}

pub async fn enable_zone_provider(this_device: Option<&DeviceInfo>,session_token: Option<&String>,is_gateway:bool) -> NSResult<()> {
    let mut client = NameClient::new(NameClientConfig::default());
    client.enable_zone_provider(this_device,session_token,is_gateway);
    let set_result = GLOBAL_NAME_CLIENT.set(client);
    if set_result.is_err() {
        return Err(NSError::Failed("Failed to set GLOBAL_NAME_CLIENT".to_string()));
    }
    Ok(())
}

fn get_name_client() -> Option<&'static NameClient> {
    let client = GLOBAL_NAME_CLIENT.get();
    if client.is_none() {
        let client = GLOBAL_BOOT_NAME_CLIENT.get();
        if client.is_none() {
            return None;
        }
        return client;
    }
    return client;
}

pub async fn resolve(name: &str, record_type: Option<&str>) -> NSResult<NameInfo> {
    let client = get_name_client();
    if client.is_none() {
        let client = GLOBAL_BOOT_NAME_CLIENT.get();
        if client.is_none() {
            return Err(NSError::NotFound("Name client not init".to_string()));
        }
    }
    let client = client.unwrap();
    client.resolve(name, record_type).await
}


pub async fn resolve_ed25519_auth_key(hostname: &str) -> NSResult<[u8; 32]> {
    //return #auth-key
    let did = DID::from_host_name(hostname);
    if did.is_some(){
        let did = did.unwrap();
        if let Some(auth_key) = did.get_auth_key() {
            return Ok(auth_key);
        }

        return Err(NSError::NotFound("Auth key not found".to_string()));
    }

    let client = get_name_client();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not init".to_string()));
    }
    let did_doc = client.unwrap().resolve_did(hostname,None).await?;
    //try conver did_doc to DeviceConfig
    match did_doc {
        EncodedDocument::JsonLd(value) => {
            let device_config = serde_json::from_value::<DeviceConfig>(value);
            if device_config.is_ok() {
                let device_config = device_config.unwrap();
                let auth_key = serde_json::to_value(&device_config.auth_key);
                if auth_key.is_ok() {
                    let auth_key = auth_key.unwrap();
                    let x = auth_key.get("x");
                    if x.is_some() {
                        let x = x.unwrap();
                        let x = x.as_str().unwrap();
                        let auth_key = URL_SAFE_NO_PAD.decode(x).unwrap();
                        return Ok(auth_key.try_into().unwrap());
                    }
                }
            }
            return Err(NSError::NotFound("Auth key not found".to_string()));
        }
        _ => {
            return Err(NSError::NotFound("Invalid did document".to_string()));
        }
    }
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
    async fn test_resolve_nameinfo() {
        let name_info = resolve("buckyos.io",Some("DID")).await.unwrap();
        println!("name_info: {:?}",name_info);
    }

    fn test_resolve_did() {

    }

}
