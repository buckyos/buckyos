use super::control::ControlTunnel;
use super::protocol::*;
use super::tcp::TcpTunnel;
use super::tunnel::*;
use crate::error::GatewayResult;

pub struct TunnelBuilder {
    device_id: String,
    remote_device_id: String,
}

impl TunnelBuilder {
    pub fn new(device_id: String, remote_device_id: String) -> Self {
        Self {
            device_id,
            remote_device_id,
        }
    }

    pub async fn build_control_tunnel(&self) -> GatewayResult<ControlTunnel> {
        let tunnel = TcpTunnel::build(self.remote_device_id.clone()).await?;
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
        assert!(port > 0);

        let tunnel = TcpTunnel::build(self.remote_device_id.clone()).await?;
        let (reader, mut writer) = tunnel.split();

        let build_pkg = ControlPackage::new(
            ControlCmd::Init,
            TunnelUsage::Data,
            Some(self.remote_device_id.clone()),
            Some(port),
            seq,
        );
        ControlPackageTransceiver::write_package(&mut writer, build_pkg).await?;

        Ok((reader, writer))
    }
}
