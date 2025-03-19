use super::stack::RTcpStack;
use crate::GatewayDeviceRef;
use crate::TunnelResult;
use name_lib::DID;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RTcpStackManager {
    device: GatewayDeviceRef,
    stack_map: Arc<Mutex<HashMap<String, RTcpStack>>>,
}

impl RTcpStackManager {
    pub fn new(device: GatewayDeviceRef) -> Self {
        Self {
            device,
            stack_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_rtcp_stack(&self, device_name: &str) -> Option<RTcpStack> {
        let stack_map = self.stack_map.lock().await;
        stack_map.get(device_name).cloned()
    }

    pub async fn get_current_device_stack(&self) -> TunnelResult<RTcpStack> {
        let this_device_hostname: String;
        let this_device_did = DID::from_str(self.device.config.did.as_str());
        if this_device_did.is_none() {
            this_device_hostname = self.device.config.did.clone();
        } else {
            this_device_hostname = this_device_did.unwrap().to_host_name();
        }

        let mut rtcp_stack_map = self.stack_map.lock().await;
        let rtcp_stack = rtcp_stack_map.get(this_device_hostname.as_str());
        if rtcp_stack.is_some() {
            let ret = rtcp_stack.unwrap().clone();
            return Ok(ret);
        }

        info!(
            "create current device rtcp stack for {}",
            this_device_hostname.as_str()
        );

        info!(
            "RTCP stack will init by this_device_config: {:?}",
            self.device.config
        );

        let mut result_rtcp_stack = crate::RTcpStack::new(
            this_device_hostname.clone(),
            2980,
            Some(self.device.private_key.clone()),
        );
        result_rtcp_stack.start().await?;

        rtcp_stack_map.insert(this_device_hostname.clone(), result_rtcp_stack.clone());

        return Ok(result_rtcp_stack);
    }
}
