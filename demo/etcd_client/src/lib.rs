use std::os::linux::raw::stat;
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

pub fn start_etcd(name: &str, url: &str) -> std::io::Result<Child> {
    // let name = "default"; // node id?
    let initial_cluster = format!("{}={}", name, url);

    Command::new("etcd")
        .arg("--name")
        .arg(name)
        .arg("--initial-cluster")
        .arg(initial_cluster)
        .arg("--initial-advertise-peer-urls")
        .arg(url)
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
    name: &str,
    url: &str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let etcd_child = start_etcd(name, url)?;
    sleep(Duration::from_secs(5)).await;

    let client = EtcdClient::connect("http://localhost:2379").await?;

    let revision = client.get_revision().await?;

    stop_etcd(etcd_child)?;

    Ok(revision)
}

pub async fn try_restore_etcd(file: &str, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    // 首先确保 etcd 服务不在运行
    stop_etcd_service()?;

    let name = "default";
    let initial_cluster = format!("{}={}", name, url);

    let status = Command::new("etcdctl")
        .arg("snapshot")
        .arg("restore")
        .arg(file)
        .arg("--name")
        .arg(name)
        .arg("--initial-cluster")
        .arg(initial_cluster)
        .arg("--initial-advertise-peer-urls")
        .arg(url)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();

    if status.is_err() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed to restore etcd snapshot",
        )));
    }
    Ok(())
}

fn stop_etcd_service() -> std::io::Result<()> {
    // 检查 etcd 进程是否运行，适当时停止它
    let status = Command::new("pkill").arg("-f").arg("etcd").status()?;

    if status.success() {
        println!("etcd service stopped successfully.");
    } else {
        println!("No etcd service was running, or failed to stop.");
    }

    // 确保进程有足够的时间停止
    std::thread::sleep(std::time::Duration::from_secs(1));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // use std::process::Command;
    use tokio::runtime::Runtime;

    #[test]
    fn test_connect_success() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let client = EtcdClient::connect("http://127.0.0.1:2379").await;
            assert!(client.is_ok());
        });
    }

    #[tokio::test]
    async fn test_get_etcd_data_version_success() {
        let result = get_etcd_data_version("default", "http://127.0.0.1:2380").await;

        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }
}
