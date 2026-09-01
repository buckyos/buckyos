use crate::AppInstanceId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayShortcut {
    App { app_instance_id: AppInstanceId },
    System { service_id: String },
}

#[derive(Serialize, Deserialize)]
pub struct GatewaySettings {
    pub shortcuts: HashMap<String, GatewayShortcut>,
}
