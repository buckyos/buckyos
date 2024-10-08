use std::time::Duration;
use crate::{DIDDocumentTrait, EncodedDocument, NSError, NSProvider, NSResult, NameInfo};

pub struct NameQuery {
    providers: Vec<Box<dyn NSProvider>>,
}

impl NameQuery {
    pub fn new() -> NameQuery {
        NameQuery {
            providers: Vec::new(),
        }
    }

    pub fn add_provider(&mut self, provider: Box<dyn NSProvider>) {
        self.providers.push(provider);
    }

    pub async fn query(&self, name: &str,record_type:Option<&str>) -> NSResult<NameInfo> {
        if self.providers.len() == 0 {
            return Err(NSError::Failed(format!("no provider for {}", name)));
        }

        for provider in self.providers.iter() {
            match provider.query(name,record_type).await {
                Ok(info) => {
                    return Ok(info);
                },
                Err(e) => {
                    //log::error!("query err {}", e);
                    continue;
                }
            }
        }
        Err(NSError::NotFound(String::from(name)))
    }

    pub async fn query_did(&self, did: &str) -> NSResult<EncodedDocument> {
        if self.providers.len() == 0 {
            return Err(NSError::Failed(format!("no provider for {}", did)));
        }

        for provider in self.providers.iter() {
            match provider.query_did(did,None).await {
                Ok(info) => {
                    return Ok(info);
                },
                Err(e) => {
                    //log::error!("query err {}", e);
                    continue;
                }
            }
        }
        Err(NSError::NotFound(String::from(did)))
    }
}
