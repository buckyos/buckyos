use super::tunnel::RTcpTunnel;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RTcpTunnelMap {
    tunnel_map: Arc<Mutex<HashMap<String, RTcpTunnel>>>,
}

impl RTcpTunnelMap {
    pub fn new() -> Self {
        RTcpTunnelMap {
            tunnel_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn tunnel_map(&self) -> Arc<Mutex<HashMap<String, RTcpTunnel>>> {
        self.tunnel_map.clone()
    }

    pub async fn get_tunnel(&self, tunnel_key: &str) -> Option<RTcpTunnel> {
        let all_tunnel = self.tunnel_map.lock().await;
        if let Some(tunnel) = all_tunnel.get(tunnel_key) {
            Some(tunnel.clone())
        } else {
            None
        }
    }

    pub async fn on_new_tunnel(&self, tunnel_key: &str, tunnel: RTcpTunnel) {
        let mut all_tunnel = self.tunnel_map.lock().await;
        let mut_old_tunnel = all_tunnel.get(tunnel_key);
        if mut_old_tunnel.is_some() {
            warn!("tunnel {} already exist", tunnel_key);
            mut_old_tunnel.unwrap().close().await;
        }

        all_tunnel.insert(tunnel_key.to_owned(), tunnel);
    }

    pub async fn remove_tunnel(&self, tunnel_key: &str) {
        let mut all_tunnel = self.tunnel_map.lock().await;
        all_tunnel.remove(tunnel_key);
    }
}
