use log::info;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
pub struct ShortcutTarget {
    #[serde(rename = "type")]
    type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    app_id: String,
}

// services/gateway/settings
#[derive(Serialize, Deserialize)]
pub struct ZoneGatewaySettings {
    shortcuts: HashMap<String, ShortcutTarget>,
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

    pub fn get_shortcut(&self, spec_id: &str) -> Vec<String> {
        info!("get_shortcut: {}", spec_id);
        let mut shortcut_hosts = Vec::new();
        let parts = spec_id.split("@").collect::<Vec<&str>>();
        let app_id;
        let mut user_id = None;
        if parts.len() == 2 {
            app_id = parts[0].to_string();
            user_id = Some(parts[1].to_string());
        } else {
            app_id = spec_id.to_string();
        }

        for (shortcut_id, shortcut_target) in self.shortcuts.iter() {
            if shortcut_target.app_id == app_id && shortcut_target.user_id == user_id {
                info!("get_shortcut: {} -> {}", spec_id, shortcut_id);
                shortcut_hosts.push(shortcut_id.clone());
            }
        }
        shortcut_hosts
    }
}
