//! The agent's records outside its AgentRoot, in system-config under
//! `users/<owner>/agents/<agent_id>/`: `settings` (what the owner chose when
//! creating the agent), `profile` (the public card) and `info` (what this
//! Loader reports once the agent is loaded). `--dev` keeps them in memory or
//! as JSON files of a local directory.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use buckyos_api::{
    agent_info_key, agent_profile_key, agent_settings_key, AgentId, AgentProfile,
    AgentRuntimeInfo, AgentSettings, SystemConfigError,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

pub const SETTINGS: &str = "settings";
pub const PROFILE: &str = "profile";
pub const INFO: &str = "info";

#[async_trait]
pub trait AgentRecords: Send + Sync {
    /// The record `name`; `None`: never written.
    async fn get(&self, name: &str) -> Result<Option<Value>, String>;
    async fn put(&self, name: &str, value: Value) -> Result<(), String>;
}

async fn typed<T: DeserializeOwned>(records: &dyn AgentRecords, name: &str) -> Result<Option<T>, String> {
    records
        .get(name)
        .await?
        .map(|v| serde_json::from_value(v).map_err(|e| format!("agent {name}: {e}")))
        .transpose()
}

async fn put_typed<T: Serialize>(records: &dyn AgentRecords, name: &str, value: &T) -> Result<(), String> {
    let value = serde_json::to_value(value).map_err(|e| format!("agent {name}: {e}"))?;
    records.put(name, value).await
}

pub async fn settings(records: &dyn AgentRecords) -> Result<AgentSettings, String> {
    Ok(typed(records, SETTINGS).await?.unwrap_or_default())
}

pub async fn profile(records: &dyn AgentRecords) -> Result<AgentProfile, String> {
    Ok(typed(records, PROFILE).await?.unwrap_or_default())
}

pub async fn set_profile(records: &dyn AgentRecords, profile: &AgentProfile) -> Result<(), String> {
    put_typed(records, PROFILE, profile).await
}

pub async fn info(records: &dyn AgentRecords) -> Result<Option<AgentRuntimeInfo>, String> {
    typed(records, INFO).await
}

pub async fn set_info(records: &dyn AgentRecords, info: &AgentRuntimeInfo) -> Result<(), String> {
    put_typed(records, INFO, info).await
}

/// system-config of the zone. The client is taken from the runtime for
/// every call: the process's session token is renewed.
pub struct ZoneRecords {
    owner: String,
    agent_id: AgentId,
}

impl ZoneRecords {
    pub fn new(owner: &str, agent_id: &AgentId) -> Self {
        Self {
            owner: owner.to_string(),
            agent_id: agent_id.clone(),
        }
    }

    fn key(&self, name: &str) -> Result<String, String> {
        Ok(match name {
            SETTINGS => agent_settings_key(&self.owner, &self.agent_id),
            PROFILE => agent_profile_key(&self.owner, &self.agent_id),
            INFO => agent_info_key(&self.owner, &self.agent_id),
            other => return Err(format!("unknown agent record `{other}`")),
        })
    }
}

#[async_trait]
impl AgentRecords for ZoneRecords {
    async fn get(&self, name: &str) -> Result<Option<Value>, String> {
        let key = self.key(name)?;
        let client = buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| e.to_string())?
            .get_system_config_client()
            .await
            .map_err(|e| e.to_string())?;
        match client.get(&key).await {
            Ok(v) => serde_json::from_str(&v.value)
                .map(Some)
                .map_err(|e| format!("decode {key}: {e}")),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
            Err(e) => Err(format!("get {key}: {e}")),
        }
    }

    async fn put(&self, name: &str, value: Value) -> Result<(), String> {
        let key = self.key(name)?;
        let client = buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| e.to_string())?
            .get_system_config_client()
            .await
            .map_err(|e| e.to_string())?;
        client
            .set(&key, &value.to_string())
            .await
            .map(|_| ())
            .map_err(|e| format!("set {key}: {e}"))
    }
}

/// Outside a zone: `<dir>/<name>.json` when a directory is given, else
/// memory only.
#[derive(Default)]
pub struct LocalRecords {
    dir: Option<PathBuf>,
    mem: Mutex<BTreeMap<String, Value>>,
}

impl LocalRecords {
    pub fn memory() -> Self {
        Self::default()
    }

    pub fn dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: Some(dir.into()),
            mem: Mutex::new(BTreeMap::new()),
        }
    }
}

#[async_trait]
impl AgentRecords for LocalRecords {
    async fn get(&self, name: &str) -> Result<Option<Value>, String> {
        match &self.dir {
            Some(dir) => libopendan::fsutil::read_json_opt(&dir.join(format!("{name}.json")))
                .map_err(|e| e.to_string()),
            None => Ok(self.mem.lock().expect("records").get(name).cloned()),
        }
    }

    async fn put(&self, name: &str, value: Value) -> Result<(), String> {
        match &self.dir {
            Some(dir) => libopendan::fsutil::atomic_replace_json(&dir.join(format!("{name}.json")), &value)
                .map_err(|e| e.to_string()),
            None => {
                self.mem.lock().expect("records").insert(name.to_string(), value);
                Ok(())
            }
        }
    }
}
