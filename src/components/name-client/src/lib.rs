mod provider;
mod name_client;
mod name_query;
mod dns_provider;
mod zone_provider;

pub use provider::*;
pub use name_client::*;
pub use name_query::*;
pub use dns_provider::*;
pub use zone_provider::*;

use std::net::IpAddr;
use once_cell::sync::OnceCell;
use name_lib::*;


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
    let set_result = GLOBAL_NAME_CLIENT.set(client);
    if set_result.is_err() {
        return Err(NSError::Failed("Failed to set name client".to_string()));
    }
    Ok(())
}

pub async fn resolve(name: &str, record_type: Option<&str>) -> NSResult<NameInfo> {
    let client = GLOBAL_NAME_CLIENT.get();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.resolve(name, record_type).await
}

pub async fn resolve_did(did: &str,fragment:Option<&str>) -> NSResult<EncodedDocument> {
    let client = GLOBAL_NAME_CLIENT.get();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.resolve_did(did,fragment).await
}

pub async fn add_did_cache(did: &str, doc:EncodedDocument) -> NSResult<()> {
    let client = GLOBAL_NAME_CLIENT.get();
    if client.is_none() {
        return Err(NSError::NotFound("Name client not found".to_string()));
    }
    let client = client.unwrap();
    client.add_did_cache(did, doc)
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
