#![allow(dead_code)]
#![allow(unused)]

mod utility;
mod provider;
mod name_client;
mod name_query;
mod dns_provider;
mod did;
mod config;


pub use did::*;
pub use config::*;
pub use provider::*;
pub use utility::*;
pub use name_client::*;

pub async fn resolve(name: &str,record_type:Option<&str>) -> NSResult<NameInfo> {
     unimplemented!()
}

pub async fn resolve_did(did: &str) -> NSResult<EncodedDocument> {
     unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_utility() {
        assert_eq!(is_did("did:example:123456789abcdefghi"), true);
        assert_eq!(is_did("www.buckyos.org"), false);
    }

    fn test_resolve_nameinfo() {

    }

    fn test_resolve_did() {

    }

    fn test_progress() {
        //get zone_config from dns
        
        //
    }

}
