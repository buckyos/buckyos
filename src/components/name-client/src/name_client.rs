#![allow(unused)]

use core::error;

use crate::provider::RecordType;
use crate::dns_provider::{DnsProvider};
use crate::name_query::NameQuery;
use crate::zone_provider::ZoneProvider;
use crate::NameInfo;
use buckyos_kit::get_buckyos_system_etc_dir;
use name_lib::*;

use log::*;

pub struct NameClientConfig {
    enable_cache: bool,
    local_cache_dir: Option<String>,
    max_ttl: u32,
    cache_size: u64,
}

impl Default for NameClientConfig {
    fn default() -> Self {
        Self {
            enable_cache: true,
            local_cache_dir: Some(
                get_buckyos_system_etc_dir()
                    .join("did_docs")
                    .to_string_lossy()
                    .to_string(),
            ),
            max_ttl: 3600 * 24 * 7,
            cache_size: 1024 * 1024,
        }
    }
}
pub struct NameClient {
    name_query: NameQuery,
    config: NameClientConfig,
    cache: mini_moka::sync::Cache<String, NameInfo>,
    doc_cache: mini_moka::sync::Cache<String, EncodedDocument>,
}

impl NameClient {
    pub fn new(config: NameClientConfig) -> Self {
        let mut name_query = NameQuery::new();
        name_query.add_provider(Box::new(DnsProvider::new(None)));
        //name_query.add_provider(Box::new(ZoneProvider::new()));
        let cache_size = config.cache_size;

        Self {
            name_query,
            config: config,
            cache: mini_moka::sync::Cache::new(cache_size), //TODO: enable local cache?
            doc_cache: mini_moka::sync::Cache::new(cache_size),
        }
    }

    pub fn enable_zone_provider(
        &mut self,
        this_device: Option<&DeviceInfo>,
        session_token: Option<&String>,
        is_gateway: bool,
    ) {
        self.name_query.add_provider(Box::new(ZoneProvider::new(
            this_device,
            session_token,
            is_gateway,
        )));
    }

    pub fn add_did_cache(&self, did: &str, doc: EncodedDocument) -> NSResult<()> {
        self.doc_cache.insert(did.to_string(), doc);
        Ok(())
    }

    pub fn add_nameinfo_cache(&self, name: &str, info: NameInfo) -> NSResult<()> {
        self.cache.insert(name.to_string(), info);
        Ok(())
    }

    pub async fn resolve(&self, name: &str, record_type: Option<RecordType>) -> NSResult<NameInfo> {
        if self.config.enable_cache {
            let cache_info = self.cache.get(&name.to_string());
            if cache_info.is_some() {
                return Ok(cache_info.unwrap().clone());
            }
        }

        let name_info = self.name_query.query(name, record_type).await?;

        if name_info.ttl.is_some() {
            let mut ttl = name_info.ttl.clone().unwrap();
            if ttl > self.config.max_ttl {
                ttl = self.config.max_ttl;
            }
            self.cache.insert(name.to_string(), name_info.clone());
        }

        return Ok(name_info);
    }

    pub async fn resolve_did(
        &self,
        did: &str,
        fragment: Option<&str>,
    ) -> NSResult<EncodedDocument> {
        if self.config.enable_cache {
            let cache_info = self.doc_cache.get(&did.to_string());

            if cache_info.is_some() {
                return Ok(cache_info.unwrap().clone());
            }
        }

        let did_doc = self.name_query.query_did(did).await;
        if did_doc.is_err() {
            // Try load from local cache
            if self.config.local_cache_dir.is_some() {
                let cache_dir = self.config.local_cache_dir.as_ref().unwrap();
                let file_path = format!("{}/{}.doc.json", cache_dir, did);

                info!("try load did doc from local cache: {}", file_path);
                let ret = std::fs::read_to_string(file_path.as_str());
                match ret {
                    Ok(did_doc) => {
                        let ret = serde_json::from_str::<DeviceConfig>(&did_doc);
                        match ret {
                            Ok(did_doc) => {
                                info!("load did doc from local cache: {}", file_path);
                                let did_doc_value = serde_json::to_value(&did_doc).unwrap();
                                let encoded_doc = EncodedDocument::JsonLd(did_doc_value);
                                return Ok(encoded_doc);
                            }
                            Err(e) => {
                                error!(
                                    "Parse did doc from local cache failed: {}, {}",
                                    file_path, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        error!("load did doc from local cache failed: {}, {}", file_path, e);
                    }
                }
            }
            return did_doc;
        }

        let did_doc = did_doc.unwrap();
        self.doc_cache.insert(did.to_string(), did_doc.clone());
        return Ok(did_doc);
    }
}
