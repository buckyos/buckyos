use std::vec;

use etcd_rs::{Client, ClientConfig, Endpoint, KeyValueOp};
use serde_json::json;
use std::process::{Child, Command};
use tokio::fs::write;
use tokio::time::{sleep, Duration};

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

    async fn get_revision(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let response = self.client.get_all().await?;
        Ok(response.header.revision())
    }
}

fn start_etcd(initial_cluster: &str) -> std::io::Result<Child> {
    Command::new("etcd")
        .arg("--initial-cluster")
        .arg(initial_cluster)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
}

fn stop_etcd(mut child: Child) -> std::io::Result<()> {
    child.kill()?;
    child.wait()?; // Ensure the child process terminates cleanly
    Ok(())
}

// 获取 etcd 数据版本
pub async fn get_etcd_data_version(
    initial_cluster: &str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let etcd_child = start_etcd(initial_cluster)?;
    sleep(Duration::from_secs(5)).await;

    let client = EtcdClient::connect("localhost:2379").await?;
    let revision = client.get_revision().await?;

    stop_etcd(etcd_child)?;

    Ok(revision)
}
