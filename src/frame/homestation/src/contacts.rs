//! Friends and contact tags come from Message Center (§3.2); HomeStation keeps no
//! authoritative copy, only a short-lived cache. The one thing it writes back is the
//! `homestation.following` tag: a projection of its own follows, so My Network shows them.

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
    /// The owner's groupings, used as audience and mute groups: its own user tags and
    /// the source groups (`type:<user type>`, `group:<id>`) of projected Zone users.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Carries the `homestation.following` tag.
    #[serde(default)]
    pub following: bool,
    /// The picture Message Center holds for this identity (the Zone's public one for Zone users and Agents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

#[async_trait]
pub trait Contacts: Send + Sync {
    async fn list(&self) -> Result<Vec<ContactInfo>, String>;
    /// Add or remove the `homestation.following` tag of `did`.
    async fn set_following(&self, did: &str, following: bool) -> Result<(), String>;
}

pub struct StaticContacts(pub Mutex<Vec<ContactInfo>>);

#[async_trait]
impl Contacts for StaticContacts {
    async fn list(&self) -> Result<Vec<ContactInfo>, String> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn set_following(&self, did: &str, following: bool) -> Result<(), String> {
        let mut list = self.0.lock().unwrap();
        match list.iter_mut().find(|c| c.did == did) {
            Some(contact) => contact.following = following,
            None if following => list.push(ContactInfo {
                did: did.to_string(),
                name: did.to_string(),
                friend: false,
                blocked: false,
                tags: vec![],
                following: true,
                avatar: None,
            }),
            None => {}
        }
        Ok(())
    }
}

/// A Message Center contact as HomeStation uses it. Groups are the owner's user tags plus
/// the source groups a projected Zone user comes from.
pub fn contact_info(contact: buckyos_api::Contact) -> ContactInfo {
    use buckyos_api::{contact_tag_kind, AccessGroupLevel, ContactTagKind, FriendState, CONTACT_TAG_HOMESTATION_FOLLOWING};
    ContactInfo {
        did: contact.did.to_string(),
        name: contact.name,
        friend: contact.friend_state == FriendState::Friend,
        blocked: contact.access_level == AccessGroupLevel::Block,
        following: contact.tags.iter().any(|t| t == CONTACT_TAG_HOMESTATION_FOLLOWING),
        tags: contact
            .tags
            .into_iter()
            .filter(|t| contact_tag_kind(t) == ContactTagKind::User || contact.projected_from.contains(t))
            .collect(),
        avatar: contact.avatar,
    }
}

pub struct MsgCenterContacts {
    pub owner: String,
}

#[async_trait]
impl Contacts for MsgCenterContacts {
    async fn list(&self) -> Result<Vec<ContactInfo>, String> {
        use buckyos_api::{get_buckyos_api_runtime, ContactQuery};
        let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        let client = runtime.get_msg_center_client().await.map_err(|e| e.to_string())?;
        let owner = name_lib::DID::from_str(&self.owner).map_err(|e| e.to_string())?;
        let mut contacts = Vec::new();
        let mut offset = 0u64;
        loop {
            let query = ContactQuery { limit: Some(200), offset: Some(offset), ..Default::default() };
            let page = client.list_contacts(query, Some(owner.clone())).await.map_err(|e| e.to_string())?;
            let len = page.len();
            contacts.extend(page.into_iter().map(contact_info));
            if len < 200 {
                break;
            }
            offset += len as u64;
        }
        Ok(contacts)
    }

    async fn set_following(&self, did: &str, following: bool) -> Result<(), String> {
        use buckyos_api::{get_buckyos_api_runtime, ContactUpdateTagsReq, CONTACT_TAG_HOMESTATION_FOLLOWING};
        let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        let client = runtime.get_msg_center_client().await.map_err(|e| e.to_string())?;
        let tag = vec![CONTACT_TAG_HOMESTATION_FOLLOWING.to_string()];
        let (add, remove) = if following { (tag, vec![]) } else { (vec![], tag) };
        let req = ContactUpdateTagsReq {
            did: name_lib::DID::from_str(did).map_err(|e| e.to_string())?,
            add,
            remove,
            contact_mgr_owner: Some(name_lib::DID::from_str(&self.owner).map_err(|e| e.to_string())?),
        };
        client.contact_update_tags(req).await.map(|_| ()).map_err(|e| e.to_string())
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

    /// Mirror a follow into Message Center; a failure leaves the follow itself intact.
    pub async fn set_following(&self, did: &str, following: bool) {
        if let Err(e) = self.inner.set_following(did, following).await {
            log::warn!("following tag of {did} not updated: {e}");
        }
        self.invalidate().await;
    }

    pub async fn get(&self, did: &str) -> Option<ContactInfo> {
        self.list().await.into_iter().find(|c| c.did == did)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_source_groups_count_as_audience_groups() {
        let contact: buckyos_api::Contact = serde_json::from_value(serde_json::json!({
            "did": "did:web:cust.z.example", "name": "Cust", "source": "shared", "is_verified": true,
            "access_level": "allow", "friend_state": "friend",
            "tags": ["zone_user", "group:customer", "vip", "homestation.following"],
            "projected_from": ["group:customer"], "created_at": 1, "updated_at": 1
        }))
        .unwrap();
        let info = contact_info(contact);
        assert_eq!(info.tags, vec!["group:customer".to_string(), "vip".to_string()]);
        assert!(info.friend && info.following && !info.blocked);
        let spec = crate::audience::AudienceSpec::Group { group_id: "group:customer".into() };
        let relations = crate::audience::ReaderRelations { follower: false, friend: info.friend, groups: info.tags };
        assert!(crate::audience::allows(&spec, &crate::audience::Reader::Did(info.did), &relations));
    }
}
