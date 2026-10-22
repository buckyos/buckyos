//! Users of this zone (§4.4): every one of them has a HomeStation — an own stream, inbox and
//! private state — served by this one service, like Message Center serves every user.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneUser {
    /// Zone username: the `/home/<user>` and `/homestation/<user>` path segment.
    pub user: String,
    pub did: String,
    #[serde(default)]
    pub name: String,
    /// Admin and User accounts may list their posts in the zone feed (§4.6); Limited
    /// accounts may not. The service setting `zone_feed_writers` overrides this.
    #[serde(default = "yes")]
    pub zone_feed_writer: bool,
}

fn yes() -> bool {
    true
}

#[async_trait]
pub trait UserSource: Send + Sync {
    async fn load(&self) -> Result<Vec<ZoneUser>, String>;
}

pub struct StaticUsers(pub std::sync::Mutex<Vec<ZoneUser>>);

#[async_trait]
impl UserSource for StaticUsers {
    async fn load(&self) -> Result<Vec<ZoneUser>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
}

/// `users/<id>/doc` (the DID), `users/<id>/profile` (display name) and `users/<id>/settings`
/// (type, state). Only active accounts other than the Zone owner record (Root) have a
/// HomeStation; records of an unknown type are skipped.
pub struct SystemConfigUsers;

#[async_trait]
impl UserSource for SystemConfigUsers {
    async fn load(&self) -> Result<Vec<ZoneUser>, String> {
        let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        load_users(runtime.get_system_config_client().await.map_err(|e| e.to_string())?).await
    }
}

/// The zone's users in system config. The DID is the one `users/<id>/doc` binds, never the
/// editable profile's.
pub async fn load_users(client: std::sync::Arc<buckyos_api::SystemConfigClient>) -> Result<Vec<ZoneUser>, String> {
    use serde_json::Value;
    let accounts = buckyos_api::ControlPanelClient::from_shared(client.clone());
    let ids = client.list("users").await.map_err(|e| e.to_string())?;
    let mut users = Vec::new();
    for id in ids {
        if !crate::protocol::valid_user_segment(&id) {
            continue;
        }
        let Ok(did) = accounts.get_user_did(&id).await else { continue };
        let Some(settings) = client
            .get(&format!("users/{id}/settings"))
            .await
            .ok()
            .and_then(|v| serde_json::from_str::<buckyos_api::UserSettings>(&v.value).ok())
        else {
            continue;
        };
        if !settings.is_active() {
            continue;
        }
        let profile = client.get(&format!("users/{id}/profile")).await.ok().and_then(|v| serde_json::from_str::<Value>(&v.value).ok()).unwrap_or(Value::Null);
        let name = ["display_name", "displayName", "name"]
            .iter()
            .find_map(|k| profile.get(*k).and_then(Value::as_str))
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(&id)
            .to_string();
        let zone_feed_writer = matches!(settings.user_type, buckyos_api::UserType::Admin | buckyos_api::UserType::User);
        users.push(ZoneUser { user: id.clone(), did: did.to_string(), name, zone_feed_writer });
    }
    Ok(users)
}

/// Cached view of the zone's users. A lookup miss reloads, at most every few seconds, so a
/// newly created user is served without a restart.
pub struct UserRegistry {
    source: Box<dyn UserSource>,
    ttl: Duration,
    cache: tokio::sync::Mutex<Option<(Instant, Vec<ZoneUser>)>>,
}

const MISS_RELOAD: Duration = Duration::from_secs(5);

impl UserRegistry {
    pub fn new(source: Box<dyn UserSource>, ttl: Duration) -> Self {
        Self { source, ttl, cache: tokio::sync::Mutex::new(None) }
    }

    async fn load(&self, max_age: Duration) -> Vec<ZoneUser> {
        let mut cache = self.cache.lock().await;
        if let Some((at, users)) = cache.as_ref() {
            if at.elapsed() < max_age {
                return users.clone();
            }
        }
        match self.source.load().await {
            Ok(users) => {
                *cache = Some((Instant::now(), users.clone()));
                users
            }
            Err(e) => {
                log::warn!("zone users unavailable: {e}");
                cache.as_ref().map(|(_, u)| u.clone()).unwrap_or_default()
            }
        }
    }

    pub async fn list(&self) -> Vec<ZoneUser> {
        self.load(self.ttl).await
    }

    async fn find(&self, pred: impl Fn(&ZoneUser) -> bool) -> Option<ZoneUser> {
        if let Some(user) = self.list().await.into_iter().find(&pred) {
            return Some(user);
        }
        self.load(MISS_RELOAD.min(self.ttl)).await.into_iter().find(pred)
    }

    pub async fn by_user(&self, user: &str) -> Option<ZoneUser> {
        self.find(|u| u.user == user).await
    }

    /// No reload on a miss: most DIDs asked about belong to other zones.
    pub async fn by_did(&self, did: &str) -> Option<ZoneUser> {
        self.list().await.into_iter().find(|u| u.did == did)
    }

    /// For a DID that should be ours (hosted under this zone): reload on a miss.
    pub async fn by_did_fresh(&self, did: &str) -> Option<ZoneUser> {
        self.find(|u| u.did == did).await
    }
}
