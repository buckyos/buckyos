use bucky_name_service::{DNSProvider, NameInfo, NameQuery, NSResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Etcd {
    pub name: String,
    pub addr: String,
    pub port: u16,
    pub ad_port: u16,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ZoneConfig {
    pub etcds: Vec<Etcd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_server: Option<String>
}

pub struct NameClient {
    name_query: NameQuery
}

impl NameClient {
    pub fn new() -> Self {
        let mut name_query = NameQuery::new();
        name_query.add_provider(Box::new(DNSProvider::new()));
        Self {
            name_query
        }
    }

    pub async fn query(&self, name: &str) -> NSResult<NameInfo> {
        self.name_query.query(name).await
    }
}
