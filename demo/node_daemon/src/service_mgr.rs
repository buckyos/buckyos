use crate::run_item::*;
use async_trait::async_trait;
use serde_json::Value;
pub struct ServiceItem {
    name: String,
    version: String,
}

#[async_trait]
impl RunItemControl for ServiceItem {
    async fn deploy(&self,params:Option<&Value>) -> Result<()> {
        Ok(())
    }

    async fn remove(&self,params:Option<&Value>) -> Result<()> {
        Ok(())
    }

    async fn update(&self,params:Option<&Value>) -> Result<String> {
        Ok(String::from("1.0.1"))
    }

    async fn start(&self,params:Option<&Value>) -> Result<()> {
        Ok(())
    }

    async fn stop(&self,params:Option<&Value>) -> Result<()> {
        Ok(())
    }

    async fn get_state(&self) -> Result<RunItemState> {
        Ok(RunItemState::Started)
    }
}

pub struct ServiceMgr {
    services: Vec<ServiceItem>,
}