use serde::{Deserialize, Serialize};

use crate::{DIDDocumentTrait, EncodedDocument, NSProvider, NSResult, NameInfo};
use crate::dns_provider::DNSProvider;  
use crate::name_query::NameQuery;


pub struct NameClientConfig {
    enable_cache: bool,
    local_cache_dir: Option<String>,
    max_ttl: u64,
    cache_size:u64,
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
        let cache_size = config.cache_size;

        Self { 
            name_query, 
            config:config,
            cache: mini_moka::sync::Cache::new(cache_size),//TODO: enable local cache?
            doc_cache: mini_moka::sync::Cache::new(cache_size),
        }
    }

    pub async fn add_did_cache(&self, did: &String, doc:EncodedDocument) -> NSResult<()> {
        self.doc_cache.insert(did.clone(), doc);
        Ok(())
    }

    pub async fn reslove(&self, name: &String,record_type:Option<&str>) -> NSResult<NameInfo> {
        if self.config.enable_cache {
            let cache_info = self.cache.get(name);
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
            self.cache.insert(name.clone(), name_info.clone());
        } 
      
        return Ok(name_info);
    }

    pub async fn resolve_did(&self, did: &String,fragment:Option<&str>) -> NSResult<EncodedDocument> {
        if self.config.enable_cache {
            let cache_info = self.doc_cache.get(did);
            if cache_info.is_some() {
                return Ok(cache_info.unwrap().clone());
            }
        }

        let did_doc = self.name_query.query_did(did).await?;

        self.doc_cache.insert(did.clone(), did_doc.clone());
        return Ok(did_doc);
    }
}

