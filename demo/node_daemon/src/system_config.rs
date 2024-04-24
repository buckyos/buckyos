use etcd_client::*;

pub struct SystemConfig {
    client: Option<EtcdClient>,
}

impl SystemConfig {
    pub fn new(client: Option<EtcdClient>) -> Self {
        SystemConfig { client }
    }

    pub async fn list(&self, prefix: &str) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        unimplemented!();
    }

    pub async fn get(&self, key: &str) -> Result<(String, i64), Box<dyn std::error::Error>> {
        unimplemented!();
    }

    pub async fn put(&self, key: &str, value: &str) -> Result<i64, Box<dyn std::error::Error>> {
        unimplemented!();
    }
}
