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
    stack_map: Arc<Mutex<HashMap<DID, RTcpStack>>>,
}

impl RTcpStackManager {
    pub fn new(device: GatewayDeviceRef) -> Self {
        Self {
            device,
            stack_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_rtcp_stack(&self, device_did: &DID) -> Option<RTcpStack> {
        let stack_map = self.stack_map.lock().await;
        stack_map.get(device_did).cloned()
    }

    pub async fn get_current_device_stack(&self) -> TunnelResult<RTcpStack> {

        let mut rtcp_stack_map = self.stack_map.lock().await;
        let rtcp_stack = rtcp_stack_map.get(&self.device.config.id);
        if rtcp_stack.is_some() {
            let ret = rtcp_stack.unwrap().clone();
            return Ok(ret);
        }

        info!(
            "RTCP stack will init by {}, device_config: {:?}",
            self.device.config.id.to_host_name(),
            self.device.config
        );

        let mut result_rtcp_stack = crate::RTcpStack::new(
            self.device.config.id.clone(),
            2980,
            Some(self.device.private_key.clone()),
        );
        result_rtcp_stack.start().await?;

        rtcp_stack_map.insert(self.device.config.id.clone(), result_rtcp_stack.clone());

        info!("RTCP stack init success");

        return Ok(result_rtcp_stack);
    }
}
