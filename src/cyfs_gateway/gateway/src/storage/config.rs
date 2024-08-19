use super::storage::*;
use crate::config::ConfigLoader;
use crate::peer::NameManagerRef;
use crate::proxy::ProxyManagerRef;
use crate::service::UpstreamManagerRef;
use gateway_lib::*;

use std::sync::{Arc, Mutex};

pub struct ConfigStorage {
    storage: StorageRef,

    name_manager: NameManagerRef,
    upstream_manager: UpstreamManagerRef,
    proxy_manager: ProxyManagerRef,

    current_value: Mutex<Option<serde_json::Value>>,
}

pub type ConfigStorageRef = Arc<ConfigStorage>;

impl ConfigStorage {
    pub fn new(
        storage: StorageRef,
        name_manager: NameManagerRef,
        upstream_manager: UpstreamManagerRef,
        proxy_manager: ProxyManagerRef,
    ) -> Self {
        Self {
            storage,
            name_manager,
            upstream_manager,
            proxy_manager,

            current_value: Mutex::new(None),
        }
    }

    pub fn notify_config_change(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            let _ = this.save().await;
        });
    }

    pub async fn save(&self) -> GatewayResult<()> {
        let upstream_list = self.upstream_manager.dump();
        let proxy_list = self.proxy_manager.dump();
        let service_list = [upstream_list, proxy_list].concat();

        let json = serde_json::json!({
            "service": service_list,
        });

        // Check if the config is changed
        {
            let current_value = self.current_value.lock().unwrap();
            if let Some(ref value) = *current_value {
                if value == &json {
                    return Ok(());
                }
            }
        }

        self.storage.save(&json).await?;

        // Update current value
        {
            let mut current_value = self.current_value.lock().unwrap();
            *current_value = Some(json);
        }

        info!("Save config success");

        Ok(())
    }

    pub async fn load(&self) -> GatewayResult<()> {
        let json = self.storage.load().await?;
        if json.is_none() {
            return Ok(());
        }

        let json = json.unwrap();

        let loader = ConfigLoader::new(
            self.name_manager.clone(),
            self.upstream_manager.clone(),
            self.proxy_manager.clone(),
        );

        loader.load(&json)?;

        {
            let mut current_value = self.current_value.lock().unwrap();
            *current_value = Some(json);
        }

        Ok(())
    }
}
