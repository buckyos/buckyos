//! Friends and contact tags come from Message Center (§3.2); HomeStation keeps no
//! authoritative copy, only a short-lived cache for display. The one thing it writes back is
//! the `homestation.following` tag: a projection of its own follows, so My Network shows them.

use crate::error::{denied, HsResult};
use async_trait::async_trait;
use buckyos_api::DenyReason;
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
    async fn get(&self, did: &str) -> Result<Option<ContactInfo>, String> {
        Ok(self.list().await?.into_iter().find(|c| c.did == did))
    }
    /// Is `did` in the owner's group `tag` as decided now (projected Zone groups included)?
    async fn is_member(&self, did: &str, tag: &str) -> Result<bool, String> {
        Ok(self.get(did).await?.is_some_and(|c| !c.blocked && c.tags.iter().any(|t| t == tag)))
    }
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

    async fn get(&self, did: &str) -> Result<Option<ContactInfo>, String> {
        let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        let client = runtime.get_msg_center_client().await.map_err(|e| e.to_string())?;
        let did = name_lib::DID::from_str(did).map_err(|e| e.to_string())?;
        let owner = name_lib::DID::from_str(&self.owner).map_err(|e| e.to_string())?;
        let contact = client.get_contact(did, Some(owner)).await.map_err(|e| e.to_string())?;
        Ok(contact.map(contact_info))
    }

    /// Message Center answers projected groups from the accounts as they are now, not from
    /// the tags of its last zone sync.
    async fn is_member(&self, did: &str, tag: &str) -> Result<bool, String> {
        let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        let client = runtime.get_msg_center_client().await.map_err(|e| e.to_string())?;
        let req = buckyos_api::ContactCheckMembershipReq {
            owner: name_lib::DID::from_str(&self.owner).map_err(|e| e.to_string())?,
            did: name_lib::DID::from_str(did).map_err(|e| e.to_string())?,
            tag: tag.to_string(),
        };
        client.contact_check_membership(req).await.map(|r| r.member).map_err(|e| e.to_string())
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

/// Names, pictures and groupings for display are cached briefly and refreshed on Message
/// Center change events. Authorization (who may read, who is blocked, who gets restricted
/// posts) reads Message Center on every decision instead: a revoked relation takes effect on
/// the next request, and an unreachable Message Center refuses rather than answering from an
/// old copy (D-5).
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

    /// The relation with `did` as Message Center holds it now, for an authorization decision.
    pub async fn current(&self, did: &str) -> HsResult<Option<ContactInfo>> {
        self.inner.get(did).await.map_err(|e| {
            log::warn!("relation with {did} unavailable: {e}");
            denied(DenyReason::AuthorizationUnavailable, "contacts")
        })
    }

    /// Group membership of `did` decided now, for an authorization decision.
    pub async fn member_now(&self, did: &str, tag: &str) -> HsResult<bool> {
        self.inner.is_member(did, tag).await.map_err(|e| {
            log::warn!("membership of {did} in {tag} unavailable: {e}");
            denied(DenyReason::AuthorizationUnavailable, "contacts")
        })
    }

    /// The whole address book as Message Center holds it now.
    pub async fn current_list(&self) -> HsResult<Vec<ContactInfo>> {
        match self.inner.list().await {
            Ok(contacts) => {
                *self.cache.lock().await = Some((Instant::now(), contacts.clone()));
                Ok(contacts)
            }
            Err(e) => {
                log::warn!("contacts unavailable: {e}");
                Err(denied(DenyReason::AuthorizationUnavailable, "contacts"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audience::Reader;
    use crate::error::HsError;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A Message Center that can be taken down.
    struct Flaky {
        up: Arc<AtomicBool>,
        book: Arc<Mutex<Vec<ContactInfo>>>,
    }

    #[async_trait]
    impl Contacts for Flaky {
        async fn list(&self) -> Result<Vec<ContactInfo>, String> {
            match self.up.load(Ordering::SeqCst) {
                true => Ok(self.book.lock().unwrap().clone()),
                false => Err("message center unreachable".into()),
            }
        }

        async fn set_following(&self, _did: &str, _following: bool) -> Result<(), String> {
            Ok(())
        }
    }

    /// alice's station with one post for `audience`, reading relations from `contacts`.
    async fn station_with_post(contacts: Box<dyn Contacts>, audience: serde_json::Value) -> (tempfile::TempDir, Arc<crate::Station>) {
        let dir = tempfile::tempdir().unwrap();
        let (pem, x) = crate::sign::generate_key();
        let owner = "did:test:alice";
        let mut directory = crate::directory::StaticDirectory::default();
        directory.identities.insert(
            owner.into(),
            crate::directory::StaticIdentity { zone: Some("alice.test".into()), user: Some("alice".into()), public_key_x: Some(x), ..Default::default() },
        );
        let mut cfg = crate::StationConfig::new(owner, "alice", "alice.test", "alice");
        cfg.spider_enabled = false;
        let signer = crate::sign::Signer::from_pem(pem.as_bytes(), format!("{owner}#main_key")).unwrap();
        let cross_zone = Arc::new(buckyos_api::CrossZoneClient::new(signer.device_signer().unwrap(), None).unwrap());
        let station = crate::Station::new(
            cfg,
            crate::db::Db::open(&dir.path().join("homestation-main.db")).unwrap().for_owner("alice"),
            signer,
            Arc::new(directory),
            cross_zone,
            CachedContacts::new(contacts, Duration::from_secs(30)),
            Arc::new(crate::objects::Storage::nfs_server(&crate::test_nfs::start_nfs_server(&dir.path().join("nfs")).await)),
            None,
        );
        let input = serde_json::from_value(json!({ "text": "for some", "attachments": [], "link": null, "audience": audience })).unwrap();
        assert_eq!(station.publish("p1", input).await.unwrap().stage, "published");
        (dir, station)
    }

    #[tokio::test]
    async fn authorization_reads_relations_now_and_refuses_when_they_are_unknown() {
        let up = Arc::new(AtomicBool::new(true));
        let friend = ContactInfo { did: "did:test:bob".into(), name: "bob".into(), friend: true, blocked: false, tags: vec![], following: false, avatar: None };
        let book = Arc::new(Mutex::new(vec![friend]));
        let (_dir, station) = station_with_post(Box::new(Flaky { up: up.clone(), book: book.clone() }), json!({ "kind": "friends" })).await;
        let bob = Reader::Did("did:test:bob".into());
        let visible = |reader: Reader| {
            let station = station.clone();
            async move { station.read_display(&reader, None, None, 10, false).await.map(|page| page.items.len()) }
        };
        assert_eq!(visible(bob.clone()).await.unwrap(), 1);

        // Message Center down: names stay cached for display, authorization refuses.
        up.store(false, Ordering::SeqCst);
        assert!(station.contacts.get("did:test:bob").await.is_some());
        for result in [visible(bob.clone()).await.map(|_| ()), station.relation("did:test:bob").await.map(|_| ())] {
            match result {
                Err(HsError::Denied(denial)) => assert_eq!(denial.reason, DenyReason::AuthorizationUnavailable),
                other => panic!("expected authorization_unavailable, got {other:?}"),
            }
        }
        assert_eq!(visible(Reader::Anonymous).await.unwrap(), 0);
        assert_eq!(visible(Reader::Owner).await.unwrap(), 1);

        // Back without the friendship: the next read already goes by it, cache TTL or not.
        book.lock().unwrap().clear();
        up.store(true, Ordering::SeqCst);
        assert_eq!(visible(bob).await.unwrap(), 0);
    }

    /// Message Center whose contact tags are a stale zone sync while membership is decided
    /// from the accounts now.
    struct Projected {
        book: Vec<ContactInfo>,
        members: Arc<Mutex<Vec<(String, String)>>>,
        up: Arc<AtomicBool>,
    }

    #[async_trait]
    impl Contacts for Projected {
        async fn list(&self) -> Result<Vec<ContactInfo>, String> {
            Ok(self.book.clone())
        }

        async fn is_member(&self, did: &str, tag: &str) -> Result<bool, String> {
            if !self.up.load(Ordering::SeqCst) {
                return Err("membership unavailable".into());
            }
            Ok(self.members.lock().unwrap().iter().any(|(d, t)| d == did && t == tag))
        }

        async fn set_following(&self, _did: &str, _following: bool) -> Result<(), String> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn group_audiences_ask_membership_now_not_the_synced_tags() {
        let up = Arc::new(AtomicBool::new(true));
        let members = Arc::new(Mutex::new(vec![("did:test:bob".to_string(), "group:customer".to_string())]));
        // The last zone sync still lists bob in group:customer.
        let bob_contact = ContactInfo { did: "did:test:bob".into(), name: "bob".into(), friend: false, blocked: false, tags: vec!["group:customer".into()], following: false, avatar: None };
        let contacts = Projected { book: vec![bob_contact], members: members.clone(), up: up.clone() };
        let (_dir, station) = station_with_post(Box::new(contacts), json!({ "kind": "group", "groupId": "group:customer" })).await;
        let bob = Reader::Did("did:test:bob".into());
        let visible = || {
            let station = station.clone();
            let bob = bob.clone();
            async move { station.read_display(&bob, None, None, 10, false).await.map(|page| page.items.len()) }
        };
        assert_eq!(visible().await.unwrap(), 1);
        assert_eq!(station.audience_recipients(&crate::audience::AudienceSpec::Group { group_id: "group:customer".into() }).await, vec!["did:test:bob".to_string()]);

        // Removed from the group: the next read drops the post although the synced tag stays.
        members.lock().unwrap().clear();
        assert_eq!(visible().await.unwrap(), 0);
        assert!(station.audience_recipients(&crate::audience::AudienceSpec::Group { group_id: "group:customer".into() }).await.is_empty());

        // Membership cannot be confirmed: refused, not answered from the tags.
        members.lock().unwrap().push(("did:test:bob".into(), "group:customer".into()));
        up.store(false, Ordering::SeqCst);
        match visible().await {
            Err(HsError::Denied(denial)) => assert_eq!(denial.reason, DenyReason::AuthorizationUnavailable),
            other => panic!("expected authorization_unavailable, got {other:?}"),
        }
    }

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
