//! The default conversation of an owner with a peer (`Message Center.md`
//! §5.8.1).
//!
//! The owner picks one of the conversations they already have with the peer:
//! a native session of their own, or a tunnel conversation, which only lives
//! in the peer's mailbox. What is kept is the conversation — the endpoint the
//! peer replies to and its session id, as in the source of a notification —
//! so a tunnel that moves on to another UI session of the Agent stays the
//! default. It is stored as owner-scoped UI state under a name of its own
//! (`default:<peer>`).
//!
//! There always is one: getting it picks the owner's earliest chat with the
//! peer when none was set, and creates one when there is no chat or the
//! default was deleted. An Agent's notifications go there as well, so sending
//! one gets it too.

use std::collections::HashSet;

use crate::contact_mgr::ContactMgr;
use crate::msg_center::MessageCenter;
use crate::notify::Conversation;
use buckyos_api::{
    DefaultSession, MsgCenterCreateSessionReq, MsgCenterGetDefaultSessionReq,
    MsgCenterListConversationsReq, MsgCenterSetDefaultSessionReq, PeerConversation,
    PeerConversations, SessionLifecycle, SessionMemoryMode,
};
use kRPC::RPCErrors;
use log::{info, warn};
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_SESSION_KEY: &str = "default_session";
const PLATFORM_NATIVE: &str = "native";
/// Owner-scoped UI state of a session an App started (its id).
const APP_SOURCE_KEY: &str = "ui.source";

type Result<T> = std::result::Result<T, RPCErrors>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Stored {
    Set(DefaultSession),
    /// The default was deleted: the next one is created.
    Deleted { deleted_at_ms: u64 },
}

fn state_session(peer: &DID) -> String {
    format!("default:{}", peer.to_string())
}

fn own_session_id(peer: &DID, to_session: Option<&str>) -> String {
    to_session
        .map(str::to_string)
        .unwrap_or_else(|| format!("dm:{}", peer.to_string()))
}

impl MessageCenter {
    async fn stored_default(&self, owner: &DID, peer: &DID) -> Result<Option<Stored>> {
        let entry = self
            .msg_box_db
            .get_owner_ui_session_state(owner, &state_session(peer), DEFAULT_SESSION_KEY)
            .await?;
        let stored = entry.and_then(|entry| serde_json::from_value(entry.value).ok());
        // Deleted before the deletion was written down here.
        if let Some(Stored::Set(default)) = &stored {
            if default.platform == PLATFORM_NATIVE
                && self.session_deleted(owner, peer, &default.session_id).await?
            {
                return Ok(Some(Stored::Deleted {
                    deleted_at_ms: Self::now_ms(),
                }));
            }
        }
        Ok(stored)
    }

    /// Deleted by `owner`, and no message came after that.
    async fn session_deleted(&self, owner: &DID, peer: &DID, session_id: &str) -> Result<bool> {
        let state = self.msg_box_db.get_owner_session(owner, session_id).await?;
        Ok(state.is_some_and(|s| !s.registered && s.deleted_at_ms.is_some())
            && !self.owns_session_with(owner, peer, session_id).await?)
    }

    async fn save_default(&self, owner: &DID, state_session: &str, value: &Stored) -> Result<()> {
        self.msg_box_db
            .upsert_owner_ui_session_state(
                owner,
                state_session,
                DEFAULT_SESSION_KEY,
                &json!(value),
                Self::now_ms(),
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn list_conversations_internal(
        &self,
        req: MsgCenterListConversationsReq,
    ) -> Result<PeerConversations> {
        let conversations = self
            .agent_conversations(&req.peer_did, &req.owner)
            .await?
            .into_iter()
            .map(|c| PeerConversation {
                session_id: c.session_id,
                platform: c.platform,
                to: c.to,
                to_session: c.to_session,
                last_active_at_ms: c.last_active_at_ms,
            })
            .collect();
        let default = match self.stored_default(&req.owner, &req.peer_did).await? {
            Some(Stored::Set(default)) => Some(default),
            _ => None,
        };
        Ok(PeerConversations {
            default,
            conversations,
        })
    }

    pub(crate) async fn get_default_session_internal(
        &self,
        req: MsgCenterGetDefaultSessionReq,
    ) -> Result<DefaultSession> {
        let _guard = self.default_lock.lock().await;
        let default = self.resolve_default(&req.owner, &req.peer_did).await?;
        if !req.native || default.platform == PLATFORM_NATIVE {
            return Ok(default);
        }
        let session_id = self.main_session(&req.owner, &req.peer_did).await?;
        Ok(Self::native_default(&req.owner, &req.peer_did, session_id))
    }

    /// The default, made when there is none. The caller holds `default_lock`.
    async fn resolve_default(&self, owner: &DID, peer: &DID) -> Result<DefaultSession> {
        let session_id = match self.stored_default(owner, peer).await? {
            Some(Stored::Set(default)) => return Ok(default),
            Some(Stored::Deleted { .. }) => self.new_native_session(owner, peer).await?,
            None => self.main_session(owner, peer).await?,
        };
        let default = Self::native_default(owner, peer, session_id);
        self.save_default(owner, &state_session(peer), &Stored::Set(default.clone()))
            .await?;
        info!(
            "default session of {} with {}: {}",
            owner.to_string(),
            peer.to_string(),
            default.session_id
        );
        Ok(default)
    }

    fn native_default(owner: &DID, peer: &DID, session_id: String) -> DefaultSession {
        DefaultSession {
            to: owner.to_string(),
            to_session: Some(session_id.clone()).filter(|s| s != &own_session_id(peer, None)),
            session_id,
            platform: PLATFORM_NATIVE.to_string(),
            set_at_ms: Self::now_ms(),
        }
    }

    /// The native chat that is the main one while the owner set none: the
    /// earliest the owner started (active ones first, registered ones first),
    /// a new one when there is none.
    async fn main_session(&self, owner: &DID, peer: &DID) -> Result<String> {
        let apps: HashSet<String> = self
            .msg_box_db
            .list_owner_ui_session_states_by_key(owner, APP_SOURCE_KEY)
            .await?
            .into_iter()
            .filter(|s| s.value.as_str().is_some_and(|source| !source.is_empty()))
            .map(|s| s.session_id)
            .collect();
        let states = self.msg_box_db.list_owner_sessions(owner).await?;
        let mut own: Vec<_> = states
            .iter()
            .filter(|s| {
                s.registered
                    && s.peer_did.as_ref() == Some(peer)
                    && !apps.contains(&s.session_id)
                    && s.binding.as_ref().and_then(|b| b.get("kind")).and_then(Value::as_str)
                        != Some("tunnel")
            })
            .collect();
        own.sort_by_key(|s| {
            (
                s.lifecycle == SessionLifecycle::Archived,
                s.created_at_ms,
                s.session_id.clone(),
            )
        });
        if let Some(first) = own.first() {
            return Ok(first.session_id.clone());
        }
        // Known only from their records, which say nothing of their beginning.
        let mut loose = self.msg_box_db.list_visible_session_index(owner).await?;
        loose.retain(|e| {
            !apps.contains(&e.session_id)
                && !states.iter().any(|s| s.registered && s.session_id == e.session_id)
        });
        loose.sort_by_key(|e| {
            (
                e.session_id != own_session_id(peer, None),
                e.last_activity_ms,
                e.session_id.clone(),
            )
        });
        for entry in loose {
            if self.owns_session_with(owner, peer, &entry.session_id).await? {
                return Ok(entry.session_id);
            }
        }
        self.new_native_session(owner, peer).await
    }

    /// A chat as the owner would have started it.
    async fn new_native_session(&self, owner: &DID, peer: &DID) -> Result<String> {
        let state = self
            .create_session_internal(MsgCenterCreateSessionReq {
                owner: owner.clone(),
                peer_did: peer.clone(),
                session_id: None,
                title: None,
                binding: Some(json!({ "kind": PLATFORM_NATIVE, "targetDid": peer.to_string() })),
                origin: None,
                set_default: false,
                memory_mode: SessionMemoryMode::Normal,
            })
            .await?;
        Ok(state.session_id)
    }

    pub(crate) async fn set_default_session_internal(
        &self,
        req: MsgCenterSetDefaultSessionReq,
    ) -> Result<DefaultSession> {
        self.store_default_session(&req.owner, &req.peer_did, &req.to, req.to_session)
            .await
    }

    /// The conversation must be one the owner has with the peer.
    pub(crate) async fn store_default_session(
        &self,
        owner: &DID,
        peer: &DID,
        to: &str,
        to_session: Option<String>,
    ) -> Result<DefaultSession> {
        let native = owner.to_string() == to;
        let own_session = own_session_id(peer, to_session.as_deref());
        let found = if native && self.owns_session_with(owner, peer, &own_session).await? {
            Some((own_session, PLATFORM_NATIVE.to_string()))
        } else {
            self.agent_conversations(peer, owner)
                .await?
                .into_iter()
                .find(|c| c.to == to && c.to_session == to_session)
                .map(|c| match native {
                    true => (own_session, PLATFORM_NATIVE.to_string()),
                    false => (c.session_id, c.platform),
                })
        };
        let Some((session_id, platform)) = found else {
            return Err(RPCErrors::ReasonError(format!(
                "default-session-unknown: {} has no conversation {}/{} with {}",
                owner.to_string(),
                to,
                to_session.as_deref().unwrap_or("-"),
                peer.to_string()
            )));
        };
        let default = DefaultSession {
            session_id,
            platform,
            to: to.to_string(),
            to_session: to_session.filter(|s| !native || s != &own_session_id(peer, None)),
            set_at_ms: Self::now_ms(),
        };
        let _guard = self.default_lock.lock().await;
        self.save_default(owner, &state_session(peer), &Stored::Set(default.clone()))
            .await?;
        Ok(default)
    }

    /// A session of `owner` whose other side is `peer`.
    async fn owns_session_with(&self, owner: &DID, peer: &DID, session_id: &str) -> Result<bool> {
        let state = self.msg_box_db.get_owner_session(owner, session_id).await?;
        if let Some(state) = state.as_ref().filter(|s| s.registered) {
            return Ok(state.peer_did.as_ref() == Some(peer));
        }
        let watermark = state
            .as_ref()
            .and_then(crate::owner_session_db::DeleteWatermark::from_state);
        let records = self
            .msg_box_db
            .list_visible_session_records(owner, session_id, 1, None, None, true, watermark.as_ref())
            .await?;
        Ok(records.iter().any(|r| &r.from == peer || &r.to == peer))
    }

    /// `session_id` of the owner is deleted: where it was the default, the
    /// next one is created when it is asked for.
    pub(crate) async fn forget_default_session(&self, owner: &DID, session_id: &str) -> Result<()> {
        let _guard = self.default_lock.lock().await;
        for entry in self
            .msg_box_db
            .list_owner_ui_session_states_by_key(owner, DEFAULT_SESSION_KEY)
            .await?
        {
            let Ok(Stored::Set(default)) = serde_json::from_value::<Stored>(entry.value) else {
                continue;
            };
            if default.platform == PLATFORM_NATIVE && default.session_id == session_id {
                let deleted = Stored::Deleted {
                    deleted_at_ms: Self::now_ms(),
                };
                self.save_default(owner, &entry.session_id, &deleted).await?;
            }
        }
        Ok(())
    }

    /// Where an Agent's notifications go besides their source conversation:
    /// the default. While its tunnel is gone, the main native chat.
    pub(crate) async fn default_conversation(
        &self,
        owner: &DID,
        agent: &DID,
    ) -> Result<Conversation> {
        let _guard = self.default_lock.lock().await;
        let mut default = self.resolve_default(owner, agent).await?;
        if default.platform != PLATFORM_NATIVE {
            let routed = DID::from_str(&default.to)
                .ok()
                .and_then(|did| ContactMgr::parse_msgtunnel_did(&did))
                .is_some_and(|(_, _, instance)| self.lookup_tunnel_route(&instance).is_some());
            if !routed {
                warn!(
                    "default conversation {} of {} with {} has no tunnel",
                    default.session_id,
                    owner.to_string(),
                    agent.to_string()
                );
                let session_id = self.main_session(owner, agent).await?;
                default = Self::native_default(owner, agent, session_id);
            }
        }
        // A native conversation is the same session in the Agent's mailbox,
        // except the one without a session id.
        let session_id = match (default.platform.as_str(), &default.to_session) {
            (PLATFORM_NATIVE, None) => format!("dm:{}", default.to),
            (PLATFORM_NATIVE, Some(session_id)) => session_id.clone(),
            _ => default.session_id,
        };
        Ok(Conversation {
            session_id,
            platform: default.platform,
            to: default.to,
            to_session: default.to_session,
            last_active_at_ms: default.set_at_ms,
        })
    }
}
