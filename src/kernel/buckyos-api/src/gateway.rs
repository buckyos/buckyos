use serde::{Serialize, Deserialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
pub struct GatewayShortcut {
    #[serde(rename = "type")]
    pub target_type: String,
    pub user_id: Option<String>,
    pub app_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct GatewaySettings {
    pub shortcuts: HashMap<String, GatewayShortcut>,
}