use crate::config::GlobalConfig;
use crate::peer::{NameManagerRef, PeerManagerRef};
use crate::proxy::ProxyManagerRef;
use crate::service::UpstreamManagerRef;

use tokio::sync::OnceCell;

pub struct GlobalContext {
    config: GlobalConfig,
    name_manager: OnceCell<NameManagerRef>,
    peer_manager: OnceCell<PeerManagerRef>,
    upstream_manager: OnceCell<UpstreamManagerRef>,
    proxy_manager: OnceCell<ProxyManagerRef>,
}

impl GlobalContext {
    pub fn new(config: GlobalConfig) -> Self {
        Self {
            config,
            name_manager: OnceCell::new(),
            peer_manager: OnceCell::new(),
            upstream_manager: OnceCell::new(),
            proxy_manager: OnceCell::new(),
        }
    }

    pub fn config(&self) -> &GlobalConfig {
        &self.config
    }

    pub fn name_manager(&self) -> &NameManagerRef {
        self.name_manager.get().unwrap()
    }

    pub fn peer_manager(&self) -> &PeerManagerRef {
        self.peer_manager.get().unwrap()
    }

    pub fn upstream_manager(&self) -> &UpstreamManagerRef {
        self.upstream_manager.get().unwrap()
    }

    pub fn proxy_manager(&self) -> &ProxyManagerRef {
        self.proxy_manager.get().unwrap()
    }
}
