struct PeerClient {
    device_id: String,
    control_tunnel: Box<dyn Tunnel>,
}

impl PeerClient {
    pub async fn new(control_tunnel: Box<dyn Tunnel>) -> GatewayResult<Self> {
        Ok(Self {
            control_tunnel,
        })
    }

    pub async fn start(&mut self) -> GatewayResult<()> {
        let build_pkg = ControlPackageTransceiver::read_package(&mut self.control_tunnel).await?;
        match build_pkg.cmd {
            ControlCommand::Build => {
                let build_pkg = ControlPackage::from_json(&build_pkg.data)?;
                let tunnel = TcpTunnel::new(build_pkg.id, self.control_tunnel);
                tunnel.start().await?;
            }
            _ => {
                error!("Invalid control command: {:?}", build_pkg.cmd);
            }
        }
        Ok(())
    }
}

struct PeerManager {

}