use std::sync::Arc;
use tokio::sync::RwLock;
use name_lib::*;

use crate::{NsProvider,NameInfo, RecordType};

pub struct NameQuery {
    providers: Arc<RwLock<Vec<Box<dyn NsProvider>>>>,
}

impl NameQuery {
    pub fn new() -> NameQuery {
        NameQuery {
            providers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn add_provider(&self, provider: Box<dyn NsProvider>) {
        let mut providers = self.providers.write().await;
        providers.push(provider);
    }

    pub async fn query(&self, name: &str,record_type:Option<RecordType>) -> NSResult<NameInfo> {
        let providers = self.providers.read().await;
        if providers.len() == 0 {
            let msg = format!("No provider found for {}", name);
            error!("{}", msg);
            return Err(NSError::Failed(msg));
        }

        let record_type = record_type.unwrap_or_default();

        for provider in providers.iter().rev() {
            match provider.query(name,Some(record_type),None).await {
                Ok(info) => {
                    info!("Resolved {} to {:?}", name, info);
                    return Ok(info);
                },
                Err(_e) => {
                    //log::error!("query err {}", e);
                    continue;
                }
            }
        }
        Err(NSError::NotFound(String::from(name)))
    }

    pub async fn query_did(&self, did: &DID) -> NSResult<EncodedDocument> {
        let providers = self.providers.read().await;
        if providers.len() == 0 {
            return Err(NSError::Failed(format!("no provider for {}", did.to_host_name())));
        }

        for provider in providers.iter() {
            match provider.query_did(did,None,None).await {
                Ok(info) => {
                    return Ok(info);
                },
                Err(_e) => {
                    //log::error!("query err {}", e);
                    continue;
                }
            }
        }
        Err(NSError::NotFound(did.to_host_name()))
    }
}
