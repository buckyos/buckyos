#![allow(unused)]

use name_lib::*;
use crate::dns_provider::DNSProvider;  
use crate::zone_provider::ZoneProvider;
use crate::name_query::NameQuery;
use crate::NameInfo;


pub struct NameClientConfig {
    enable_cache: bool,
    local_cache_dir: Option<String>,
    max_ttl: u32,
    cache_size:u64,
}

impl Default for NameClientConfig {
    fn default() -> Self {
        Self {
            enable_cache: true,
            local_cache_dir: None,
            max_ttl: 3600*24*7,
            cache_size: 1024*1024,
        }
    }
}
pub struct NameClient {
    name_query: NameQuery,
    config : NameClientConfig,
    cache: mini_moka::sync::Cache<String, NameInfo>,
    doc_cache: mini_moka::sync::Cache<String, EncodedDocument>
}

impl NameClient {
    pub fn new(config:NameClientConfig) -> Self {
        let mut name_query = NameQuery::new();
        name_query.add_provider(Box::new(DNSProvider::new(None)));
        //name_query.add_provider(Box::new(ZoneProvider::new()));
        let cache_size = config.cache_size;

        Self { 
            name_query, 
            config:config,
            cache: mini_moka::sync::Cache::new(cache_size),//TODO: enable local cache?
            doc_cache: mini_moka::sync::Cache::new(cache_size),
        }
    }

    pub fn enable_zone_provider(&mut self,this_device: Option<&DeviceInfo>,session_token: Option<&String>,is_gateway:bool) {
        self.name_query.add_provider(Box::new(ZoneProvider::new(this_device,session_token,is_gateway)));
    }

    pub fn add_did_cache(&self, did: &str, doc:EncodedDocument) -> NSResult<()> {
        self.doc_cache.insert(did.to_string(), doc);
        Ok(())
    }

    pub async fn resolve(&self, name: &str,record_type:Option<&str>) -> NSResult<NameInfo> {
        if self.config.enable_cache {
            let cache_info = self.cache.get(&name.to_string());
            if cache_info.is_some() {
                return Ok(cache_info.unwrap().clone());
            }
        }

        let name_info = self.name_query.query(name,record_type).await?;
 
        if name_info.ttl.is_some() {
            let mut ttl = name_info.ttl.clone().unwrap();
            if ttl > self.config.max_ttl {
                ttl = self.config.max_ttl;
            }
            self.cache.insert(name.to_string(), name_info.clone());
        } 
      
        return Ok(name_info);
    }

    pub async fn resolve_did(&self, did: &str,fragment:Option<&str>) -> NSResult<EncodedDocument> {
        if self.config.enable_cache {
            let cache_info = self.doc_cache.get(&did.to_string());

            if cache_info.is_some() {
                return Ok(cache_info.unwrap().clone());
            }
        }

        let did_doc = self.name_query.query_did(did).await?;

        self.doc_cache.insert(did.to_string(), did_doc.clone());
        return Ok(did_doc);
    }
}

