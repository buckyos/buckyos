#![allow(unused)]

use core::error;

use crate::provider::RecordType;
use crate::dns_provider::{DnsProvider};
use crate::name_query::NameQuery;
use crate::{NameInfo, NsProvider};
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
    doc_cache: mini_moka::sync::Cache<DID, EncodedDocument>,
}

impl NameClient {
    pub fn new(config: NameClientConfig) -> Self {
        let mut name_query = NameQuery::new();
        //name_query.add_provider(Box::new(DnsProvider::new(None)));
        //name_query.add_provider(Box::new(ZoneProvider::new()));
        let cache_size = config.cache_size;

        Self {
            name_query,
            config: config,
            cache: mini_moka::sync::Cache::new(cache_size), //TODO: enable local cache?
            doc_cache: mini_moka::sync::Cache::new(cache_size),
        }
    }

    pub async fn add_provider(&self, provider: Box<dyn NsProvider>) {
        self.name_query.add_provider(provider).await;
    } 

    pub fn add_did_cache(&self, did: DID, doc: EncodedDocument) -> NSResult<()> {
        self.doc_cache.insert(did, doc);
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
        let mut real_name = name.to_string();
        if name.starts_with("did") {
            let name_did = DID::from_str(name);
            if name_did.is_ok() {
                let name_did = name_did.unwrap();
                if name_did.method.as_str() == "web" {
                    info!("resolve did:web is some as resolve host: {}", name_did.id.as_str());
                    real_name = name_did.id.clone();
                }
            }
        }

        let name_info = self.name_query.query(real_name.as_str(), record_type).await?;
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
        did: &DID,
        fragment: Option<&str>,
    ) -> NSResult<EncodedDocument> {
        if self.config.enable_cache {
            let cache_info = self.doc_cache.get(did);

            if cache_info.is_some() {
                return Ok(cache_info.unwrap().clone());
            }
        }

        let did_doc = self.name_query.query_did(did).await;
        if did_doc.is_err() {
            // Try load from local cache
            if self.config.local_cache_dir.is_some() {
                let cache_dir = self.config.local_cache_dir.as_ref().unwrap();
                // let file_path = format!("{}/{}.doc.json", cache_dir, did);
                let mut file_path = std::path::PathBuf::new();
                file_path.push(cache_dir);
                file_path.push(format!("{}.doc.json", did.to_host_name()));
                let file_path = file_path.to_str().unwrap().to_string();

                debug!("try load did doc from local cache: {}", file_path);
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
        self.doc_cache.insert(did.clone(), did_doc.clone());
        return Ok(did_doc);
    }
}
