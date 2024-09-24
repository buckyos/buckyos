#![allow(dead_code)]
#![allow(unused)]

mod utility;
mod provider;
mod name_client;
mod name_query;
mod dns_provider;
mod did;
mod config;

use once_cell::sync::Lazy;
use tokio::sync::Mutex;

static GLOBAL_NAME_CLIENT: Lazy<Mutex<NameClient>> = Lazy::new(|| {
    Mutex::new(NameClient::new(NameClientConfig::default()))
});

pub use did::*;
pub use config::*;
pub use provider::*;
pub use dns_provider::*;
pub use utility::*;
pub use name_client::*;

pub async fn resolve(name: &str, record_type: Option<&str>) -> NSResult<NameInfo> {
    let client = GLOBAL_NAME_CLIENT.lock().await;
    client.resolve(name, record_type).await
}

pub async fn resolve_did(did: &str,fragment:Option<&str>) -> NSResult<EncodedDocument> {
    let client = GLOBAL_NAME_CLIENT.lock().await;
    client.resolve_did(did,fragment).await
}

pub async fn add_did_cache(did: &str, doc:EncodedDocument) -> NSResult<()> {
    let client = GLOBAL_NAME_CLIENT.lock().await;
    client.add_did_cache(did, doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_utility() {
        assert_eq!(is_did("did:example:123456789abcdefghi"), true);
        assert_eq!(is_did("www.buckyos.org"), false);
    }

    #[tokio::test]
    async fn test_resolve_nameinfo() {
        let name_info = resolve("buckyos.io",Some("DID")).await.unwrap();
        println!("name_info: {:?}",name_info);
    }

    fn test_resolve_did() {

    }

    fn test_progress() {
        //get zone_config from dns
        
        //
    }

}
