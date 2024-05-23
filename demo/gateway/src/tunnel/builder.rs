use super::control::ControlTunnel;
use super::protocol::*;
use super::tcp::TcpTunnel;
use super::tunnel::*;
use crate::error::*;
use crate::peer::{NameInfo, NameManagerRef};

pub struct TunnelBuilder {
    name_manager: NameManagerRef,
    device_id: String,
    remote_device_id: String,
}

impl TunnelBuilder {
    pub fn new(name_manager: NameManagerRef, device_id: String, remote_device_id: String) -> Self {
        Self {
            name_manager,
            device_id,
            remote_device_id,
        }
    }

    async fn resolve_remote(&self) -> GatewayResult<NameInfo> {
        let remote = self.name_manager.resolve(&self.remote_device_id).await;
        if remote.is_none() {
            return Err(GatewayError::PeerNotFound(format!(
                "Peer not found: {}",
                self.remote_device_id
            )));
        }
        Ok(remote.unwrap())
    }

    pub async fn build_control_tunnel(&self) -> GatewayResult<ControlTunnel> {
        let remote = self.resolve_remote().await?;

        let tunnel = TcpTunnel::build(remote.addr).await?;
        let (reader, mut writer) = tunnel.split();

        let init_pkg = ControlPackage::new(
            ControlCmd::Init,
            TunnelUsage::Control,
            Some(self.device_id.clone()),
            None,
            0,
        );
        ControlPackageTransceiver::write_package(&mut writer, init_pkg).await?;

        Ok(ControlTunnel::new(
            TunnelSide::Active,
            self.device_id.clone(),
            self.remote_device_id.clone(),
            reader,
            writer,
        ))
    }

    pub async fn build_data_tunnel(
        &self,
        port: u16,
        seq: u32,
    ) -> GatewayResult<(Box<dyn TunnelReader>, Box<dyn TunnelWriter>)> {
        info!(
            "Will build data tunnel: {} -> {}, port={}, seq={}",
            self.device_id, self.remote_device_id, port, seq
        );

        assert!(port > 0);

        let remote = self.resolve_remote().await?;
        let tunnel = TcpTunnel::build(remote.addr).await?;
        let (reader, mut writer) = tunnel.split();

        let build_pkg = ControlPackage::new(
            ControlCmd::Init,
            TunnelUsage::Data,
            Some(self.device_id.clone()),
            Some(port),
            seq,
        );
        ControlPackageTransceiver::write_package(&mut writer, build_pkg).await?;

        Ok((reader, writer))
    }
}
