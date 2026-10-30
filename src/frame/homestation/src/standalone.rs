//! Standalone mode: one zone's HomeStation service from a JSON config — the zone owner and any
//! other users of the zone, the zone key that signs for all of them, peers, contacts, tokens,
//! and the zone's standalone nfs-server (named store and buckets, started with
//! `--bucket-specs` declaring `homestation`). The database is `<data-dir>/homestation-main.db`.
//! Used by `homestation --data-dir --config` and by the multi-node tests.

use crate::auth::{StaticIdentity, StaticTokens};
use crate::contacts::{CachedContacts, ContactInfo, StaticContacts};
use crate::db::Db;
use crate::directory::{StaticDirectory, StaticIdentity as DirIdentity};
use crate::error::{HsError, HsResult};
use crate::evaluation::ModelClient;
use crate::host::{Host, HostConfig};
use crate::http::AppState;
use crate::objects::Storage;
use crate::settings::UserSettings;
use crate::sign::Signer;
use crate::standalone_hub::StandaloneHub;
use crate::users::{StaticUsers, UserRegistry, ZoneUser};
use crate::zone::ZoneFeed;
use crate::{Station, StationConfig};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Intervals {
    pub pull: Option<u64>,
    pub delivery: Option<u64>,
    pub select: Option<u64>,
    pub comments: Option<u64>,
}

/// Another user of the zone: an own home, contacts and settings.
#[derive(Clone, Deserialize)]
pub struct StandaloneUser {
    pub user: String,
    pub did: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub contacts: Vec<ContactInfo>,
    #[serde(default)]
    pub settings: Option<UserSettings>,
    #[serde(default)]
    pub collector: bool,
    #[serde(default)]
    pub zone_feed_writer: Option<bool>,
    /// A zone Agent: `{ agent_id, owner_did }`.
    #[serde(default)]
    pub agent: Option<crate::users::ZoneAgentRef>,
}

#[derive(Clone, Deserialize)]
pub struct StandaloneConfig {
    pub owner: String,
    #[serde(default)]
    pub owner_name: Option<String>,
    /// The owner's username (`/home/<user>`); defaults to `owner`.
    #[serde(default)]
    pub user: Option<String>,
    pub zone: String,
    /// Base URL of the zone's nfs-server (`http://127.0.0.1:<port>`).
    pub nfs_server: String,
    #[serde(default)]
    pub zone_name: Option<String>,
    /// The zone key: signs for every user of the zone (kid defaults to `<owner>#main_key`).
    #[serde(default)]
    pub private_key_pem: Option<String>,
    #[serde(default)]
    pub private_key_file: Option<String>,
    #[serde(default)]
    pub kid: Option<String>,
    #[serde(default)]
    pub listen: Option<String>,
    #[serde(default)]
    pub collector: bool,
    #[serde(default)]
    pub spider: Option<bool>,
    #[serde(default)]
    pub workers: Option<bool>,
    #[serde(default)]
    pub disclose_admission: Option<bool>,
    #[serde(default)]
    pub intervals_s: Intervals,
    #[serde(default)]
    pub reading_window: Option<usize>,
    #[serde(default)]
    pub tokens: HashMap<String, StaticIdentity>,
    #[serde(default)]
    pub directory: StaticDirectory,
    /// The owner's contacts.
    #[serde(default)]
    pub contacts: Vec<ContactInfo>,
    /// The owner's initial settings, applied when none are stored yet.
    #[serde(default)]
    pub settings: Option<UserSettings>,
    /// Other users of the zone.
    #[serde(default)]
    pub users: Vec<StandaloneUser>,
    /// `<user>` or `~zone`.
    #[serde(default)]
    pub default_feed: Option<String>,
    #[serde(default)]
    pub zone_feed_writers: Option<Vec<String>>,
}

pub struct Standalone {
    pub host: Arc<Host>,
    /// The zone's users as the host reads them (tests add and remove Agents here).
    pub users: Arc<StaticUsers>,
    /// The owner's station.
    pub station: Arc<Station>,
    pub state: AppState,
    /// The owner's contacts.
    pub contacts: Arc<StaticContacts>,
    /// Every user's contacts, by username.
    pub user_contacts: HashMap<String, Arc<StaticContacts>>,
    pub listen: String,
}

struct SharedContacts(Arc<StaticContacts>);

#[async_trait::async_trait]
impl crate::contacts::Contacts for SharedContacts {
    async fn list(&self) -> Result<Vec<ContactInfo>, String> {
        self.0.list().await
    }

    async fn set_following(&self, did: &str, following: bool) -> Result<(), String> {
        self.0.set_following(did, following).await
    }
}

struct SharedUsers(Arc<StaticUsers>);

#[async_trait::async_trait]
impl crate::users::UserSource for SharedUsers {
    async fn load(&self) -> Result<Vec<ZoneUser>, String> {
        self.0.load().await
    }
}

struct UserSetup {
    contacts: Arc<StaticContacts>,
    settings: Option<UserSettings>,
    collector: bool,
}

pub fn build(cfg: StandaloneConfig, data_dir: &Path, config_dir: Option<&Path>, model: Option<Arc<dyn ModelClient>>) -> HsResult<Standalone> {
    let store = Arc::new(Storage::nfs_server(&cfg.nfs_server));
    let db = Db::open(&data_dir.join("homestation-main.db"))?;
    let pem = match (&cfg.private_key_pem, &cfg.private_key_file) {
        (Some(pem), _) => pem.clone(),
        (None, Some(file)) => {
            let path = config_dir.map(|d| d.join(file)).unwrap_or_else(|| file.into());
            std::fs::read_to_string(&path).map_err(|e| HsError::Internal(format!("read key {}: {e}", path.display())))?
        }
        (None, None) => return Err(HsError::BadRequest("config needs private_key_pem or private_key_file".into())),
    };
    let kid = cfg.kid.clone().unwrap_or_else(|| format!("{}#main_key", cfg.owner));
    let signer = Signer::from_pem(pem.as_bytes(), kid).map_err(HsError::BadRequest)?;
    let zone = cfg.zone.to_ascii_lowercase();
    let owner_user = cfg.user.clone().unwrap_or_else(|| "owner".into());
    let listen = cfg.listen.clone().unwrap_or_else(|| "127.0.0.1:4130".into());

    let mut zone_users = vec![ZoneUser {
        user: owner_user.clone(),
        did: cfg.owner.clone(),
        name: cfg.owner_name.clone().unwrap_or_else(|| "Owner".into()),
        zone_feed_writer: true,
        agent: None,
    }];
    let owner_contacts = Arc::new(StaticContacts(Mutex::new(cfg.contacts.clone())));
    let mut setups: HashMap<String, UserSetup> = HashMap::new();
    setups.insert(owner_user.clone(), UserSetup { contacts: owner_contacts.clone(), settings: cfg.settings.clone(), collector: cfg.collector });
    for u in &cfg.users {
        if !crate::protocol::valid_user_segment(&u.user) || u.user == crate::protocol::ZONE_FEED || setups.contains_key(&u.user) {
            return Err(HsError::BadRequest(format!("invalid or duplicate user {}", u.user)));
        }
        zone_users.push(ZoneUser {
            user: u.user.clone(),
            did: u.did.clone(),
            name: u.name.clone().unwrap_or_else(|| u.user.clone()),
            zone_feed_writer: u.zone_feed_writer.unwrap_or(u.agent.is_none()),
            agent: u.agent.clone(),
        });
        setups.insert(u.user.clone(), UserSetup { contacts: Arc::new(StaticContacts(Mutex::new(u.contacts.clone()))), settings: u.settings.clone(), collector: u.collector });
    }

    // Every local user lives in this zone and is signed for by the zone key.
    let mut directory = cfg.directory.clone();
    for u in &zone_users {
        let identity = directory.identities.entry(u.did.clone()).or_insert_with(DirIdentity::default);
        identity.zone = Some(zone.clone());
        identity.user = Some(u.user.clone());
        if u.did != signer.signer_did && !identity.devices.contains(&signer.signer_did) {
            identity.devices.push(signer.signer_did.clone());
        }
    }
    directory.zones.entry(zone.clone()).or_insert_with(|| format!("http://{listen}"));
    let directory = Arc::new(directory);
    // This zone signs other zones' readers in (`standalone_hub.rs`); its stations sign in at theirs.
    let public_x = crate::sign::public_x_from_pem(&pem).map_err(HsError::BadRequest)?;
    let hub = Arc::new(StandaloneHub::new(&zone, signer.clone(), &public_x, directory.clone()).map_err(HsError::BadRequest)?);
    let cross_zone = Arc::new(
        buckyos_api::CrossZoneClient::new(signer.device_signer().map_err(HsError::BadRequest)?, None)
            .map_err(|e| HsError::Internal(e.to_string()))?
            .with_resolver(directory.clone())
            .with_origins(directory.zones.clone()),
    );

    let mut template = StationConfig::new(&cfg.owner, "", &zone, &owner_user);
    template.spider_enabled = cfg.spider.unwrap_or(true);
    template.disclose_admission = cfg.disclose_admission.unwrap_or(true);
    if let Some(w) = cfg.reading_window {
        template.reading_window = w;
    }
    let i = &cfg.intervals_s;
    if let Some(v) = i.pull {
        template.pull_interval = Duration::from_secs(v);
    }
    if let Some(v) = i.delivery {
        template.delivery_interval = Duration::from_secs(v);
    }
    if let Some(v) = i.select {
        template.select_interval = Duration::from_secs(v);
    }
    if let Some(v) = i.comments {
        template.comment_sync_interval = Duration::from_secs(v);
    }

    let setups = Arc::new(setups);
    let builder = {
        let directory = directory.clone();
        let setups = setups.clone();
        let signer = signer.clone();
        let cross_zone = cross_zone.clone();
        let db = db.clone();
        Box::new(move |user: &ZoneUser| -> HsResult<Arc<Station>> {
            let setup = setups.get(&user.user);
            let db = db.for_owner(&user.user);
            if let Some(initial) = setup.and_then(|s| s.settings.as_ref()) {
                db.with(|c| {
                    if crate::db::get_setting(c, "user_settings")?.is_none() {
                        crate::settings::save(c, initial)?;
                    }
                    Ok(())
                })?;
            }
            let contacts = setup.map(|s| s.contacts.clone()).unwrap_or_else(|| Arc::new(StaticContacts(Mutex::new(vec![]))));
            let cached = CachedContacts::new(Box::new(SharedContacts(contacts)), Duration::from_millis(0));
            let mut station_cfg = template.clone();
            station_cfg.owner = user.did.clone();
            station_cfg.owner_name = user.name.clone();
            station_cfg.user = user.user.clone();
            station_cfg.collector = setup.is_some_and(|s| s.collector);
            station_cfg.agent = user.agent.clone();
            Ok(Station::new(station_cfg, db, signer.clone(), directory.clone(), cross_zone.clone(), cached, store.clone(), model.clone()))
        })
    };
    let mut host_cfg = HostConfig::new(&zone);
    host_cfg.zone_name = cfg.zone_name.clone();
    host_cfg.owner_user = Some(owner_user.clone());
    host_cfg.default_feed = cfg.default_feed.clone();
    host_cfg.zone_feed_writers = cfg.zone_feed_writers.clone();
    host_cfg.start_workers = cfg.workers.unwrap_or(true);
    let static_users = Arc::new(StaticUsers(Mutex::new(zone_users.clone())));
    let registry = Arc::new(UserRegistry::new(Box::new(SharedUsers(static_users.clone())), Duration::from_millis(0)));
    let auth = Arc::new(StaticTokens { tokens: cfg.tokens.clone(), hub: Some(hub.clone()) });
    let zone_feed = ZoneFeed::new(&db);
    let host = Host::new(host_cfg, directory, auth, registry, zone_feed, builder);
    let mut station = None;
    for user in &zone_users {
        let opened = host.open(user)?;
        if user.user == owner_user {
            station = Some(opened);
        }
    }
    let user_contacts = setups.iter().map(|(k, v)| (k.clone(), v.contacts.clone())).collect();
    Ok(Standalone {
        state: AppState { host: host.clone(), hub: Some(hub) },
        host,
        users: static_users,
        station: station.expect("owner station"),
        contacts: owner_contacts,
        user_contacts,
        listen,
    })
}
