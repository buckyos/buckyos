use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::fs::File;
use crate::get_buckyos_system_etc_dir;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct BuckyOSMachineConfig {
    pub web3_bridge: HashMap<String, String>,
    pub trust_did: Vec<String>,//did
}

impl Default for BuckyOSMachineConfig {
    fn default() -> Self {
        let mut web3_bridge = HashMap::new();
        web3_bridge.insert("bns".to_string(), "web3.buckyos.org".to_string());

        Self {
            web3_bridge,
            trust_did: vec!["did:web:buckyos.org".to_string(),
            "did:web:buckyos.ai".to_string(),
            "did:web:buckyos.io".to_string()],
        }
    }
}


impl BuckyOSMachineConfig {
    pub fn load_machine_config() -> Option<Self> {
        let machine_config_path = get_buckyos_system_etc_dir().join("machine.json");
        let machine_config_file = File::open(machine_config_path);
        if machine_config_file.is_err() {
            return None;
        }
        let machine_config  = serde_json::from_reader(machine_config_file.unwrap());
        if machine_config.is_err() {
            return None;
        }
        return Some(machine_config.unwrap());
    }
}
