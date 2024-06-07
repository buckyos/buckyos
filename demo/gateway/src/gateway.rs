use crate::config::ConfigLoader;
use crate::peer::{NameManager, NameManagerRef, PeerManager, PeerManagerRef};
use crate::proxy::{ProxyManager, ProxyManagerRef};
use crate::service::{UpstreamManager, UpstreamManagerRef};
use gateway_lib::*;

use std::net::SocketAddr;
use std::sync::Arc;

pub struct Gateway {
    config: GlobalConfigRef,
    upstream_manager: UpstreamManagerRef,
    proxy_manager: ProxyManagerRef,
    name_manager: NameManagerRef,
    peer_manager: PeerManagerRef,
}

impl Gateway {
    pub fn load(json: &serde_json::Value) -> GatewayResult<Self> {
        let config = ConfigLoader::load_config_node(json)?;

        let name_manager = Arc::new(NameManager::new());
        let upstream_manager = Arc::new(UpstreamManager::new());

        let peer_manager = PeerManager::new(config.clone(), name_manager.clone());
        let peer_manager = Arc::new(peer_manager);

        let proxy_manager = ProxyManager::new(name_manager.clone(), peer_manager.clone());
        let proxy_manager = Arc::new(proxy_manager);

        // add local device to name manager
        let addr = SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, config.tunnel_server_port()));
        name_manager.add(
            config.device_id().to_owned(),
            addr,
            Some(config.addr_type()),
        );

        // load config
        let loader = ConfigLoader::new(
            name_manager.clone(),
            upstream_manager.clone(),
            proxy_manager.clone(),
        );
        loader.load(json)?;

        peer_manager
            .events()
            .bind_events(upstream_manager.clone_as_events());

        if config.addr_type() == PeerAddrType::LAN {
            let config = config.clone();
            let name_manager = name_manager.clone();
            let peer_manager = peer_manager.clone();
            tokio::spawn(async move {
                Self::init_tunnel_with_wan_peers(config, name_manager, peer_manager).await;
            });
        }

        let ret = Self {
            config,
            upstream_manager,
            proxy_manager,
            name_manager,
            peer_manager,
        };

        Ok(ret)
    }

    pub fn upstream_manager(&self) -> UpstreamManagerRef {
        self.upstream_manager.clone()
    }

    pub fn proxy_manager(&self) -> ProxyManagerRef {
        self.proxy_manager.clone()
    }

    pub async fn start(&self) -> GatewayResult<()> {
        self.peer_manager.start().await?;

        self.proxy_manager.start().await?;

        Ok(())
    }

    async fn init_tunnel_with_wan_peers(
        config: GlobalConfigRef,
        name_manager: NameManagerRef,
        peer_manager: PeerManagerRef,
    ) {
        if config.addr_type() != PeerAddrType::LAN {
            return;
        }

        let peers = name_manager.select_peers_by_type(PeerAddrType::WAN);
        for peer in peers {
            if peer.device_id == config.device_id() {
                continue;
            }

            info!("Will init tunnel with wan peer: {:?}", peer);
            if let Err(e) = peer_manager.get_or_init_peer(&peer.device_id, true).await {
                log::error!(
                    "Init tunnel with wan peer failed: {}, {:?}",
                    peer.device_id,
                    e
                );
                continue;
            }
        }
    }
}
