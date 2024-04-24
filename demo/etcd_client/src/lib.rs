use std::vec;

use etcd_rs::{Client, ClientConfig, Endpoint, KeyValueOp};
use serde_json::json;
use tokio::fs::write;

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

    // backup snapshot to local file
    pub async fn snapshot(&self) -> Result<(), Box<dyn std::error::Error>> {
        // let options = GetOptions::new().with_range("0", "z");
        let response = self.client.get_all().await?;
        // 将响应中的键值对转换为 JSON
        let kv_pairs = response
            .kvs
            .iter()
            .map(|kv| {
                json!({
                    "key": String::from(kv.key_str()),
                    "value": String::from(kv.value_str())
                })
            })
            .collect::<Vec<_>>();
        let serialized_data = serde_json::to_string_pretty(&kv_pairs)?;
        write("etcd_snapshot.json", serialized_data).await?;

        Ok(())
    }
}
