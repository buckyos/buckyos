use crate::config::{ConfigLoader, GlobalConfigRef};
use crate::error::*;
use crate::peer::{NameManager, NameManagerRef, PeerManager, PeerManagerRef};
use crate::proxy::{ProxyManager, ProxyManagerRef};
use crate::service::{UpstreamManager, UpstreamManagerRef};

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

        let ret = Self {
            config,
            upstream_manager,
            proxy_manager,
            name_manager,
            peer_manager,
        };

        Ok(ret)
    }

    pub async fn start(&self) -> GatewayResult<()> {
        self.peer_manager.start().await?;

        self.proxy_manager.start().await?;

        Ok(())
    }
}
