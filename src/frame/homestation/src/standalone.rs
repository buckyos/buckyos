//! Standalone mode: one HomeStation from a JSON config (identity, key, peers, contacts,
//! tokens). Used by `homestation --data-dir --config` and by the multi-node tests.

use crate::auth::{StaticIdentity, StaticTokens};
use crate::contacts::{CachedContacts, ContactInfo, StaticContacts};
use crate::db::Db;
use crate::directory::StaticDirectory;
use crate::error::{HsError, HsResult};
use crate::evaluation::ModelClient;
use crate::http::AppState;
use crate::objects::FsChunkStore;
use crate::settings::UserSettings;
use crate::sign::Signer;
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

#[derive(Clone, Deserialize)]
pub struct StandaloneConfig {
    pub owner: String,
    #[serde(default)]
    pub owner_name: Option<String>,
    pub zone: String,
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
    #[serde(default)]
    pub contacts: Vec<ContactInfo>,
    /// Initial user settings, applied when none are stored yet.
    #[serde(default)]
    pub settings: Option<UserSettings>,
}

pub struct Standalone {
    pub station: Arc<Station>,
    pub state: AppState,
    pub contacts: Arc<StaticContacts>,
    pub listen: String,
}

struct SharedContacts(Arc<StaticContacts>);

#[async_trait::async_trait]
impl crate::contacts::Contacts for SharedContacts {
    async fn list(&self) -> Result<Vec<ContactInfo>, String> {
        self.0.list().await
    }
}

pub fn build(cfg: StandaloneConfig, data_dir: &Path, config_dir: Option<&Path>, model: Option<Arc<dyn ModelClient>>) -> HsResult<Standalone> {
    std::fs::create_dir_all(data_dir).map_err(|e| HsError::Internal(format!("data dir: {e}")))?;
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
    let mut station_cfg = StationConfig::new(&cfg.owner, cfg.owner_name.as_deref().unwrap_or("Owner"), &cfg.zone);
    station_cfg.collector = cfg.collector;
    station_cfg.spider_enabled = cfg.spider.unwrap_or(true);
    station_cfg.disclose_admission = cfg.disclose_admission.unwrap_or(true);
    if let Some(w) = cfg.reading_window {
        station_cfg.reading_window = w;
    }
    let i = &cfg.intervals_s;
    if let Some(v) = i.pull {
        station_cfg.pull_interval = Duration::from_secs(v);
    }
    if let Some(v) = i.delivery {
        station_cfg.delivery_interval = Duration::from_secs(v);
    }
    if let Some(v) = i.select {
        station_cfg.select_interval = Duration::from_secs(v);
    }
    if let Some(v) = i.comments {
        station_cfg.comment_sync_interval = Duration::from_secs(v);
    }
    let db = Db::open(&data_dir.join("homestation.db"))?;
    if let Some(initial) = &cfg.settings {
        db.with(|c| {
            if crate::db::get_setting(c, "user_settings")?.is_none() {
                crate::settings::save(c, initial)?;
            }
            Ok(())
        })?;
    }
    let contacts = Arc::new(StaticContacts(Mutex::new(cfg.contacts.clone())));
    let cached = CachedContacts::new(Box::new(SharedContacts(contacts.clone())), Duration::from_millis(0));
    let chunks = Arc::new(FsChunkStore { dir: data_dir.join("chunks") });
    let station = Station::new(station_cfg, db, signer, Arc::new(cfg.directory.clone()), cached, chunks, model);
    let auth = Arc::new(StaticTokens { tokens: cfg.tokens.clone() });
    let state = AppState { station: station.clone(), auth };
    Ok(Standalone { station, state, contacts, listen: cfg.listen.clone().unwrap_or_else(|| "127.0.0.1:4130".into()) })
}
