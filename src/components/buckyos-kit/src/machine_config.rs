use std::collections::HashMap;
use serde::{Deserialize, Serialize};


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

