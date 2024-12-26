
use name_lib::*;
use crate::{NSProvider,NameInfo};

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
            let msg = format!("No provider found for {}", name);
            error!("{}", msg);
            return Err(NSError::Failed(msg));
        }

        let record_type = record_type.unwrap_or("A");

        for provider in self.providers.iter().rev() {
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

    pub async fn query_did(&self, did: &str) -> NSResult<EncodedDocument> {
        if self.providers.len() == 0 {
            return Err(NSError::Failed(format!("no provider for {}", did)));
        }

        for provider in self.providers.iter() {
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
        Err(NSError::NotFound(String::from(did)))
    }
}
