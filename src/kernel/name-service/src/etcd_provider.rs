use crate::{NameInfo, NSCmdRegister, NSProvider, NSResult};

pub struct ETCDProvider {
    etcd_url: String,
}

impl ETCDProvider {
    pub fn new(etcd_url: String) -> Self {
        Self { etcd_url }
    }
}

#[async_trait::async_trait]
impl NSProvider for ETCDProvider {
    async fn load(&self, cmd_register: &NSCmdRegister) -> NSResult<()> {
        todo!()
    }

    async fn query(&self, name: &str) -> NSResult<NameInfo> {
        todo!()
    }
}
