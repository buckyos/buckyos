// use etcd_client::*;
use etcd_rs::{Client, ClientConfig, Endpoint, KeyValueOp};
use log::*;

pub struct SystemConfig {
    client: Client,
}

impl SystemConfig {
    pub async fn new(etcd_servers: &Vec<String>) -> Result<Self, Box<dyn std::error::Error>> {
        let endpoints: Vec<Endpoint> = etcd_servers.iter().map(|s| Endpoint::new(s)).collect();
        let config = ClientConfig::new(endpoints);
        match Client::connect(config).await {
            Ok(client) => {
                //  cfg.auth 这个值如果是none，connect会直接返回一个OK，所以需要一个get来验证是否真的连接成功
                let result = client.get("tryconnect").await;
                match result {
                    Ok(_) => {
                        info!("Connected to etcd:{} success", etcd_servers.join(","));
                        Ok(SystemConfig { client })
                    }
                    Err(e) => {
                        error!(
                            "Failed to connect to etcd:{}, err:{}",
                            etcd_servers.join(","),
                            e
                        );
                        Err(Box::new(e))
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to connect to etcd:{}, err:{}",
                    etcd_servers.join(","),
                    e
                );
                Err(Box::new(e))
            }
        }
    }

    pub async fn list(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        unimplemented!();
    }

    pub async fn get(&self, key: &str) -> Result<(String, i64), Box<dyn std::error::Error>> {
        let response = self.client.get(key).await?;
        let revision = response.header.revision();
        let value = response.kvs[0].value_str();
        Ok((value.to_string(), revision))
    }

    pub async fn put(&self, key: &str, value: &str) -> Result<i64, Box<dyn std::error::Error>> {
        let response = self.client.put((key, value)).await?;
        Ok(response.header.revision())
    }
}
