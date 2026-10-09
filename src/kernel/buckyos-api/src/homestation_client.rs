use ::kRPC::kRPC;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const HOMESTATION_UNIQUE_ID: &str = "homestation";
pub const HOMESTATION_SERVICE_NAME: &str = "homestation";
pub const HOMESTATION_SERVICE_PORT: u16 = 4130;
pub const HOMESTATION_HTTP_PATH: &str = "/kapi/homestation";
/// Zone-level protocol paths forwarded by the gateway: `/home/` (zone index, DID lookup),
/// `/home/<user>/...` (each user's stream, entries, objects, inbox), `/home/~zone/...`.
pub const HOMESTATION_PROTOCOL_PREFIX: &str = "/home/";
/// Short hostname (`homestation.<zone>`) whose `/` opens the default feed, like `www.<zone>`.
pub const HOMESTATION_HOST_PREFIX: &str = "homestation";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HomeStationSettings {
    /// Accept and index public submissions from anyone (collector role).
    pub collector: bool,
    /// Attach the `candidate` / `preferred` admission disclosure to `accepted`.
    pub disclose_admission: bool,
    /// Zone hostname → HTTP origin, for zones not reachable at `https://<zone>`.
    pub peers: HashMap<String, String>,
    /// Logical AICC model for the `model` evaluation profile; none disables it.
    pub evaluation_model: Option<String>,
    pub spider: bool,
    pub reading_window: usize,
    /// What `www.<zone>/` and `homestation.<zone>/` open: a username, or `~zone` for the zone
    /// feed. None = the zone owner's feed.
    pub default_feed: Option<String>,
    /// Usernames that may list posts in the zone feed (`*` = all users). None = regular users
    /// (admin / root / user, not limited or guest).
    pub zone_feed_writers: Option<Vec<String>>,
    /// Display name of the zone feed; none = the zone hostname.
    pub zone_name: Option<String>,
}

impl Default for HomeStationSettings {
    fn default() -> Self {
        Self {
            collector: false,
            disclose_admission: true,
            peers: HashMap::new(),
            evaluation_model: None,
            spider: true,
            reading_window: 300,
            default_feed: None,
            zone_feed_writers: None,
            zone_name: None,
        }
    }
}

pub struct HomeStationClient {
    client: kRPC,
}

impl HomeStationClient {
    pub fn new(client: kRPC) -> Self {
        Self { client }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ::kRPC::RPCErrors> {
        self.client.call(method, params).await
    }
}
