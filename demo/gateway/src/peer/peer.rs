use super::name::NameManagerRef;
use gateway_lib::*;
use crate::tunnel::*;

pub struct PeerClient {
    device_id: String,
    remote_device_id: String,
    tunnel_manager: TunnelManager,
    name_manager: NameManagerRef,
}

impl PeerClient {
    pub fn new(
        device_id: String,
        remote_device_id: String,
        tunnel_manager_events: TunnelManagerEventsRef,
        name_manager: NameManagerRef,
    ) -> Self {
        let tunnel_manager = TunnelManager::new(
            name_manager.clone(),
            device_id.clone(),
            remote_device_id.clone(),
        );
        tunnel_manager.bind_events(tunnel_manager_events.clone());

        Self {
            device_id,
            remote_device_id,
            tunnel_manager,
            name_manager,
        }
    }

    // call on passive side
    pub async fn init_with_control_tunnel(
        &self,
        tunnel_reader: Box<dyn TunnelReader>,
        tunnel_writer: Box<dyn TunnelWriter>,
    ) {
        self.tunnel_manager
            .bind_tunnel_control(tunnel_reader, tunnel_writer)
            .await
    }

    // call on active side
    pub async fn start(&self) -> GatewayResult<()> {
        let local_name = self.name_manager.resolve(&self.device_id).await;
        if local_name.is_none() {
            error!("Local peer info not found: {}", self.device_id);
            return Err(GatewayError::PeerNotFound(self.device_id.clone()));
        }

        let remote_name = self.name_manager.resolve(&self.remote_device_id).await;
        if remote_name.is_none() {
            error!("Peer not found: {}", self.remote_device_id);
            return Err(GatewayError::PeerNotFound(self.remote_device_id.clone()));
        }

        let local_name = local_name.unwrap();
        let remote_name = remote_name.unwrap();

        if local_name.addr_type.unwrap() == PeerAddrType::LAN && remote_name.addr_type.unwrap() == PeerAddrType::WAN {
            self.tunnel_manager.start_control_tunnel();
        }

        Ok(())
    }

    pub fn remote_device_id(&self) -> &str {
        &self.remote_device_id
    }

    pub async fn build_data_tunnel(
        &self,
        port: u16,
    ) -> GatewayResult<(Box<dyn TunnelReader>, Box<dyn TunnelWriter>)> {
        self.tunnel_manager.build_data_tunnel(port).await
    }

    // recv tunnel connection from tunnel server, need handled by tunnel manager
    pub async fn on_new_data_tunnel(&self, info: TunnelInitInfo) {
        self.tunnel_manager.on_new_data_tunnel(info).await
    }
}
