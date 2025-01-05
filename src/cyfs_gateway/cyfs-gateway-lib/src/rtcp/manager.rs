use super::stack::RTcpStack;
use crate::CURRENT_DEVICE_PRIVATE_KEY;
use crate::{TunnelError, TunnelResult};
use name_lib::{CURRENT_DEVICE_CONFIG, DID};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RTcpStackManager {
    stack_map: Arc<Mutex<HashMap<String, RTcpStack>>>,
}

impl RTcpStackManager {
    pub fn new() -> Self {
        Self {
            stack_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_rtcp_stack(&self, device_name: &str) -> Option<RTcpStack> {
        let stack_map = self.stack_map.lock().await;
        stack_map.get(device_name).cloned()
    }

    pub async fn get_current_device_stack(&self) -> TunnelResult<RTcpStack> {
        let this_device_config = CURRENT_DEVICE_CONFIG.get();
        if this_device_config.is_none() {
            let msg = "CURRENT_DEVICE_CONFIG not set".to_string();
            error!("{}", msg);
            return Err(TunnelError::InvalidState(msg));
        }

        let this_device_config = this_device_config.unwrap();
        let this_device_hostname: String;
        let this_device_did = DID::from_str(this_device_config.did.as_str());
        if this_device_did.is_none() {
            this_device_hostname = this_device_config.did.clone();
        } else {
            this_device_hostname = this_device_did.unwrap().to_host_name();
        }

        let mut rtcp_stack_map = self.stack_map.lock().await;
        let rtcp_stack = rtcp_stack_map.get(this_device_hostname.as_str());
        if rtcp_stack.is_some() {
            let ret = rtcp_stack.unwrap().clone();
            return Ok(ret);
        }

        info!("create rtcp stack for {}", this_device_hostname.as_str());
        let this_device_private_key = CURRENT_DEVICE_PRIVATE_KEY.get();
        if this_device_private_key.is_none() {
            error!("CURRENT_DEVICE_PRIVATE_KEY not set!");
            return Err(TunnelError::InvalidState(
                "CURRENT_DEVICE_PRIVATE_KEY not set".to_string(),
            ));
        }

        info!(
            "RTCP stack will init by this_device_config: {:?}",
            this_device_config
        );
        let this_device_private_key = this_device_private_key.unwrap().clone();

        let mut result_rtcp_stack = crate::RTcpStack::new(
            this_device_hostname.clone(),
            2980,
            Some(this_device_private_key),
        );
        result_rtcp_stack.start().await?;

        rtcp_stack_map.insert(this_device_hostname.clone(), result_rtcp_stack.clone());

        return Ok(result_rtcp_stack);
    }
}

lazy_static::lazy_static! {
    pub static ref RTCP_STACK_MANAGER: RTcpStackManager = RTcpStackManager::new();
}
