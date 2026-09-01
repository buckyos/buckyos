use crate::AppInstanceId;
use log::info;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShortcutTarget {
    App { app_instance_id: AppInstanceId },
    System { service_id: String },
}

// services/gateway/settings
#[derive(Serialize, Deserialize)]
pub struct ZoneGatewaySettings {
    pub shortcuts: HashMap<String, ShortcutTarget>,
}

impl Default for ZoneGatewaySettings {
    fn default() -> Self {
        Self {
            shortcuts: HashMap::new(),
        }
    }
}

impl ZoneGatewaySettings {
    pub fn new() -> Self {
        Self {
            shortcuts: HashMap::new(),
        }
    }

    pub fn get_shortcut(&self, app_instance_id: &AppInstanceId) -> Vec<String> {
        info!("get_shortcut: {}", app_instance_id);
        let mut shortcut_hosts = Vec::new();
        for (shortcut_id, shortcut_target) in self.shortcuts.iter() {
            if matches!(shortcut_target, ShortcutTarget::App { app_instance_id: target } if target == app_instance_id)
            {
                info!("get_shortcut: {} -> {}", app_instance_id, shortcut_id);
                shortcut_hosts.push(shortcut_id.clone());
            }
        }
        shortcut_hosts
    }
}
