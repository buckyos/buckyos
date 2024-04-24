use etcd_client::*;

pub struct SystemConfig {
    client: EtcdClient,
}

impl SystemConfig {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        unimplemented!();
    }

    pub async fn get(&self, key: &str) -> Result<(String, i64), Box<dyn std::error::Error>> {
        unimplemented!();
    }

    pub async fn put(&self, key: &str, value: &str) -> Result<i64, Box<dyn std::error::Error>> {
        unimplemented!();
    }
}
