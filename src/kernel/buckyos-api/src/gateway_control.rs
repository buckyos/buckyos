use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize)]
struct Shortcut {
    #[serde(rename = "type")]
    type_: String,
    user_id: String,
    app_id: String,
}



#[derive(Serialize)]
struct ZoneGatewaySettings {
    shortcuts: HashMap<String, Shortcut>,
    //service_name -> app_id
    services: HashMap<String, String>,
}


