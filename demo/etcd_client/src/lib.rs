pub use etcd_rs::Member;
use etcd_rs::{Client, ClientConfig, ClusterOp, Endpoint, KeyValueOp};
use log::*;

use serde_json::json;
use serde_json::Value;
use std::fs::DirEntry;
use std::process::{Child, Command};
use std::vec;
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
            Ok(client) => {
                //  cfg.auth 这个值如果是none，connect会直接返回一个OK，所以需要一个get来验证是否真的连接成功
                let result = client.get("tryconnect").await;
                match result {
                    Ok(_) => {
                        info!("Connected to etcd:{} success", endpoint);
                        Ok(EtcdClient { client })
                    }
                    Err(e) => {
                        error!("Failed to connect to etcd:{}, err:{}", endpoint, e);
                        Err(Box::new(e))
                    }
                }
            }
            Err(e) => {
                error!("Failed to connect to etcd:{}, err:{}", endpoint, e);
                Err(Box::new(e))
            }
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

    pub async fn get(&self, key: &str) -> Result<(String, i64), Box<dyn std::error::Error>> {
        let response = self.client.get(key).await?;
        let revision = response.header.revision();
        let value = response.kvs[0].value_str();
        Ok((value.to_string(), revision))
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<i64, Box<dyn std::error::Error>> {
        let response = self.client.put((key, value)).await?;
        Ok(response.header.revision())
    }

    pub async fn members(&self) -> Result<Vec<Member>, Box<dyn std::error::Error>> {
        let response = self.client.member_list().await?;
        Ok(response.members)
    }
}

// start etcd as backend
pub fn start_etcd(
    name: &str,
    initial_cluster: &str,
    cluster_token: &str,
) -> std::io::Result<Child> {
    // TODO
    // 每次重新启动node daemon都应该，直接删除这个目录。
    // 否则受损的etcd会触发cluster ID mismatch的问题
    // 应该从backup那里拿到并恢复
    let etcd_data_dir = std::env::current_dir()
        .unwrap()
        .join(format!("{}.etcd", name));

    // if etcd_data_dir.exists() {
    //     std::fs::remove_dir_all(&etcd_data_dir)?;
    // }
    let cluster_state = if etcd_data_dir.exists() {
        if etcd_data_dir
            .read_dir()?
            .collect::<Vec<Result<DirEntry, std::io::Error>>>()
            .len()
            > 0
        {
            "existing"
        } else {
            "new"
        }
    } else {
        "new"
    };
    // let cluster_state = "new";
    info!("cluster state: {}; machine name: {}", cluster_state, name);

    Command::new("etcd")
        .arg("--name")
        .arg(name)
        .arg("--listen-peer-urls")
        .arg("http://0.0.0.0:2380")
        .arg("--listen-client-urls")
        .arg("http://0.0.0.0:2379")
        .arg("--advertise-client-urls")
        .arg(format!("http://{}:2379", name))
        .arg("--initial-cluster")
        .arg(initial_cluster)
        .arg("--initial-advertise-peer-urls")
        .arg(format!("http://{}:2380", name))
        .arg("--initial-cluster-token")
        .arg(cluster_token)
        .arg("--initial-cluster-state")
        .arg(cluster_state)
        // .arg("--log-outputs")
        // .arg(format!("/var/log/etcd.{}.log", name))
        // .stdout(std::process::Stdio::inherit())
        // .stderr(std::process::Stdio::inherit())
        .spawn()
}

pub async fn check_etcd_health(name: &str) -> bool {
    let timeout = 3;
    let url = "http://127.0.0.1:2379/health".to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout)) // 设置请求超时时间为2秒
        .build()
        .expect("Failed to build HTTP client");

    for _ in 0..3600 {
        let response = client.get(&url).send().await;
        info!("Checking etcd health: {:?}", response);
        if let Ok(resp) = response {
            if let Ok(json) = resp.json::<Value>().await {
                if json["health"] == "true" {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    false
}

// 获取 etcd 数据版本
pub async fn get_etcd_data_version() -> Result<i64, Box<dyn std::error::Error>> {
    let mut count = 0;
    let client = loop {
        match EtcdClient::connect("http://127.0.0.1:2379").await {
            Ok(client) => {
                break client;
            }
            Err(e) => {
                count += 1;
                if count > 20 {
                    return Err(e);
                }
                sleep(Duration::from_secs(5)).await;
            }
        }
    };

    let revision = client.get_revision().await?;

    // stop_etcd_service()?;

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

// 用etcdctl 生成快照，然后传到backupservice里面去
pub async fn backup_etcd(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let name = "default";
    let initial_cluster = format!("{}={}", name, url);

    let back_file = "/tmp/etcd_backup";
    let status = Command::new("etcdctl")
        .arg("snapshot")
        .arg("save")
        .arg(back_file)
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

    Ok(back_file.to_string())
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
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;

    #[test]
    fn test_connect_success() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            // 这里应该加个kill和start
            let client = EtcdClient::connect("http://127.0.0.1:2379").await;
            assert!(client.is_ok());

            // let endpoint = "http://127.0.0.1:2379";
            // let endpoints: Vec<Endpoint> = vec![Endpoint::new(endpoint)];
            // let config = ClientConfig::new(endpoints);
            // let cli = Client::connect(config).await.unwrap();
            // let res = cli.get("connect").await.unwrap();
        });
    }

    #[test]
    fn test_connect_faild() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            stop_etcd_service().unwrap();
            let client = EtcdClient::connect("http://127.0.0.1:2379").await;
            assert!(client.is_err());
        });
    }

    #[test]
    fn test_start_etcd() {
        let name = "testnode";
        let url = "http://127.0.0.1:2380";

        // 在一个新线程中启动 etcd
        let handle =
            thread::spawn(move || start_etcd(name, url, "test").expect("Failed to start etcd"));

        // 等待 etcd 启动，期间输出倒计时
        let countdown_time = 5; // 总倒计时时间（秒）
        let start_time = Instant::now();
        while start_time.elapsed().as_secs() < countdown_time {
            println!(
                "Waiting... {} seconds remaining",
                countdown_time - start_time.elapsed().as_secs()
            );
            thread::sleep(Duration::from_secs(1));
        }
        println!("Done waiting.");

        // 尝试连接到启动的 etcd 实例来验证它是否运行
        let status = Command::new("curl")
            .arg("-L")
            .arg(format!("{}/v2/keys", url))
            .output()
            .expect("Failed to execute command");

        assert!(status.status.success(), "etcd should be reachable");

        // 通过 handle 获取 etcd 进程并尝试关闭它
        let mut child = handle.join().expect("Thread panicked");
        child.kill().expect("Failed to kill etcd process");
    }

    #[tokio::test]
    async fn test_get_etcd_data_version_success() {
        // let result = get_etcd_data_version("default", "http://127.0.0.1:2380", "test").await;

        // assert!(result.is_ok());
        // assert!(result.unwrap() > 0);
    }
}
