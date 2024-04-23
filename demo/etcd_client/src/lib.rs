use std::vec;

use etcd_rs::{Client, ClientConfig, Endpoint};

pub struct EtcdClient {
    pub client: Client,
}

impl EtcdClient {
    pub async fn connect(endpoint: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let endpoints: Vec<Endpoint> = vec![Endpoint::new(endpoint)];
        let config = ClientConfig::new(endpoints);
        match Client::connect(config).await {
            Ok(client) => Ok(EtcdClient { client }),
            Err(e) => Err(Box::new(e)),
        }
    }
}
