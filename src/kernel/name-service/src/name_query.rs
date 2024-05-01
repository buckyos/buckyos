use std::time::Duration;
use crate::{NameInfo, NSError, NSErrorCode, NSProvider, NSResult};


pub struct NameQuery {
    providers: Vec<Box<dyn NSProvider>>,
    cache: mini_moka::sync::Cache<String, NameInfo>
}

impl NameQuery {
    pub fn new() -> NameQuery {
        let cache = mini_moka::sync::CacheBuilder::new(1024).time_to_live(Duration::from_secs(600)).build();
        NameQuery {
            providers: Vec::new(),
            cache,
        }
    }

    pub fn add_provider(&mut self, provider: Box<dyn NSProvider>) {
        self.providers.push(provider);
    }

    pub async fn query(&self, name: &str) -> NSResult<NameInfo> {
        if let Some(info) = self.cache.get(&name.to_string()) {
            return Ok(info);
        }

        for provider in self.providers.iter() {
            match provider.query(name).await {
                Ok(info) => {
                    self.cache.insert(name.to_string(), info.clone());
                    return Ok(info);
                }
                Err(e) => {
                    log::error!("query err {}", e);
                    continue;
                }
            }
        }
        Err(NSError::new(NSErrorCode::NotFound, format!("Name {} not found", name)))
    }
}
