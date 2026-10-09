//! Friends and contact groups come from Message Center (§3.2); HomeStation keeps no
//! authoritative copy, only a short-lived cache.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContactInfo {
    pub did: String,
    pub name: String,
    #[serde(default)]
    pub friend: bool,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub groups: Vec<String>,
}

#[async_trait]
pub trait Contacts: Send + Sync {
    async fn list(&self) -> Result<Vec<ContactInfo>, String>;
}

pub struct StaticContacts(pub Mutex<Vec<ContactInfo>>);

#[async_trait]
impl Contacts for StaticContacts {
    async fn list(&self) -> Result<Vec<ContactInfo>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
}

pub struct MsgCenterContacts {
    pub owner: String,
}

#[async_trait]
impl Contacts for MsgCenterContacts {
    async fn list(&self) -> Result<Vec<ContactInfo>, String> {
        use buckyos_api::{get_buckyos_api_runtime, AccessGroupLevel, ContactQuery};
        let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        let client = runtime.get_msg_center_client().await.map_err(|e| e.to_string())?;
        let owner = name_lib::DID::from_str(&self.owner).map_err(|e| e.to_string())?;
        let mut contacts = Vec::new();
        let mut offset = 0u64;
        loop {
            let query = ContactQuery { limit: Some(200), offset: Some(offset), ..Default::default() };
            let page = client.list_contacts(query, Some(owner.clone())).await.map_err(|e| e.to_string())?;
            let len = page.len();
            for contact in page {
                contacts.push(ContactInfo {
                    did: contact.did.to_string(),
                    name: contact.name,
                    friend: contact.access_level == AccessGroupLevel::Friend,
                    blocked: contact.access_level == AccessGroupLevel::Block,
                    groups: contact.groups,
                });
            }
            if len < 200 {
                break;
            }
            offset += len as u64;
        }
        Ok(contacts)
    }
}

/// Relations are read on every audience decision, so they are cached briefly; a
/// Message Center change takes effect within the TTL (§4.5 "关系变化后自动生效").
pub struct CachedContacts {
    inner: Box<dyn Contacts>,
    ttl: Duration,
    cache: tokio::sync::Mutex<Option<(Instant, Vec<ContactInfo>)>>,
}

impl CachedContacts {
    pub fn new(inner: Box<dyn Contacts>, ttl: Duration) -> Self {
        Self { inner, ttl, cache: tokio::sync::Mutex::new(None) }
    }

    pub async fn list(&self) -> Vec<ContactInfo> {
        let mut cache = self.cache.lock().await;
        if let Some((at, contacts)) = cache.as_ref() {
            if at.elapsed() < self.ttl {
                return contacts.clone();
            }
        }
        match self.inner.list().await {
            Ok(contacts) => {
                *cache = Some((Instant::now(), contacts.clone()));
                contacts
            }
            Err(e) => {
                log::warn!("contacts unavailable: {e}");
                cache.as_ref().map(|(_, c)| c.clone()).unwrap_or_default()
            }
        }
    }

    pub async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }

    pub async fn get(&self, did: &str) -> Option<ContactInfo> {
        self.list().await.into_iter().find(|c| c.did == did)
    }
}
