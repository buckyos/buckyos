//! Friend requests (`Contact Mgr.md` §friend requests): msg-center builds the
//! `buckyos.friend` operation messages for the owner, delivers them on the
//! native path, and applies a peer's messages to the owner's own records.
//! Each side keeps its own `friend_requests` rows and `Contact.friend_state`.

use crate::msg_center::MessageCenter;
use buckyos_api::{
    contact_changed_event_id, AccessGroupLevel, ContactChangedEvent, ContactListFriendRequestsReq,
    ContactSource, DeliveryRecord, DeliveryState, FriendDelivery, FriendDeliveryState,
    FriendMessageData, FriendProfileHint, FriendRequest, FriendRequestDecision,
    FriendRequestDirection, FriendRequestPage, FriendRequestResult, FriendRequestState,
    FriendState, MailboxKind, PostSendResult, RecipientState, FRIEND_ACTION_ACCEPT,
    FRIEND_ACTION_CANCEL, FRIEND_ACTION_REJECT, FRIEND_ACTION_REMOVE, FRIEND_ACTION_REQUEST,
    FRIEND_INTENT, FRIEND_REQUEST_COOLDOWN_MS, FRIEND_REQUEST_MESSAGE_MAX_CHARS,
    FRIEND_REQUEST_TTL_MS,
};
use kRPC::RPCErrors;
use log::{info, warn};
use name_lib::DID;
use ndn_lib::{MachineContent, MsgContent, MsgContentFormat, MsgObjKind, MsgObject, NamedObject};
use serde_json::json;
use sqlx::Row;
use std::future::Future;
use std::pin::Pin;

type Result<T> = std::result::Result<T, RPCErrors>;

const FRIEND_COLUMNS: &str = "request_id, peer_did, direction, state, message, msg_id, \
    peer_name, peer_avatar, delivery_json, created_at_ms, expires_at_ms, updated_at_ms";
const DEFAULT_FRIEND_REQUEST_LIMIT: usize = 50;
const MAX_FRIEND_REQUEST_LIMIT: usize = 200;
/// Avatars travel in the request only when small (data URLs can be large).
const MAX_PROFILE_HINT_AVATAR_CHARS: usize = 2048;

fn invalid(reason: &str) -> RPCErrors {
    RPCErrors::ParseRequestError(reason.to_string())
}

fn denied(reason: &str) -> RPCErrors {
    RPCErrors::NoPermission(reason.to_string())
}

fn db_error(error: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(format!("friend-db:{error}"))
}

/// What dispatch does with a received `buckyos.friend` message.
pub(crate) enum FriendIngress {
    /// Not for this recipient (blocked, cooling down, expired, unknown).
    Drop,
    /// Applied to the records; nothing goes into a mailbox.
    Control,
    /// A new request: written to the mailbox by the ordinary admission.
    Mailbox,
}

/// A message to send once the state change that caused it is persisted.
struct Outgoing {
    peer: DID,
    data: FriendMessageData,
    text: String,
    keep_sent: bool,
}

impl Outgoing {
    fn control(peer: &DID, action: &str, request_id: &str, text: &str) -> Self {
        Self {
            peer: peer.clone(),
            data: FriendMessageData {
                action: action.to_string(),
                request_id: request_id.to_string(),
                ..Default::default()
            },
            text: text.to_string(),
            keep_sent: false,
        }
    }
}

pub(crate) fn is_friend_message(msg: &MsgObject) -> bool {
    msg.content
        .machine
        .as_ref()
        .is_some_and(|machine| machine.intent.as_deref() == Some(FRIEND_INTENT))
}

pub(crate) fn friend_data(msg: &MsgObject) -> Option<FriendMessageData> {
    let machine = msg.content.machine.as_ref()?;
    if machine.intent.as_deref() != Some(FRIEND_INTENT) {
        return None;
    }
    serde_json::to_value(&machine.data)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn direction_name(direction: FriendRequestDirection) -> &'static str {
    direction.as_str()
}

fn parse_direction(raw: &str) -> Result<FriendRequestDirection> {
    match raw {
        "incoming" => Ok(FriendRequestDirection::Incoming),
        "outgoing" => Ok(FriendRequestDirection::Outgoing),
        other => Err(db_error(format!("unknown direction {other}"))),
    }
}

fn row_to_request(row: &sqlx::any::AnyRow) -> Result<FriendRequest> {
    let text = |name: &str| row.try_get::<Option<String>, _>(name).map_err(db_error);
    let number = |name: &str| row.try_get::<i64, _>(name).map(|v| v as u64).map_err(db_error);
    let peer: String = row.try_get("peer_did").map_err(db_error)?;
    let state: String = row.try_get("state").map_err(db_error)?;
    let direction: String = row.try_get("direction").map_err(db_error)?;
    Ok(FriendRequest {
        request_id: row.try_get("request_id").map_err(db_error)?,
        peer_did: DID::from_str(&peer).map_err(db_error)?,
        direction: parse_direction(&direction)?,
        state: FriendRequestState::parse(&state)
            .ok_or_else(|| db_error(format!("unknown state {state}")))?,
        message: text("message")?,
        msg_id: text("msg_id")?,
        peer_name: text("peer_name")?,
        peer_avatar: text("peer_avatar")?,
        delivery: text("delivery_json")?.and_then(|raw| serde_json::from_str(&raw).ok()),
        created_at_ms: number("created_at_ms")?,
        expires_at_ms: number("expires_at_ms")?,
        updated_at_ms: number("updated_at_ms")?,
    })
}

impl MessageCenter {
    async fn query_friend_requests(
        &self,
        owner: &DID,
        clause: &str,
        binds: Vec<String>,
        tail: &str,
    ) -> Result<Vec<FriendRequest>> {
        let sql = self.msg_box_db.render_sql(&format!(
            "SELECT {FRIEND_COLUMNS} FROM friend_requests WHERE owner_key = ?{clause} {tail}"
        ));
        let mut query = sqlx::query(&sql).bind(owner.to_string());
        for value in binds {
            query = query.bind(value);
        }
        query
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(db_error)?
            .iter()
            .map(row_to_request)
            .collect()
    }

    async fn friend_request_by_id(
        &self,
        owner: &DID,
        request_id: &str,
        direction: FriendRequestDirection,
    ) -> Result<Option<FriendRequest>> {
        Ok(self
            .query_friend_requests(
                owner,
                " AND request_id = ? AND direction = ?",
                vec![request_id.to_string(), direction_name(direction).to_string()],
                "ORDER BY updated_at_ms DESC LIMIT 1",
            )
            .await?
            .pop())
    }

    async fn friend_request_with(
        &self,
        owner: &DID,
        peer: &DID,
        request_id: &str,
    ) -> Result<Option<FriendRequest>> {
        Ok(self
            .query_friend_requests(
                owner,
                " AND peer_did = ? AND request_id = ?",
                vec![peer.to_string(), request_id.to_string()],
                "LIMIT 1",
            )
            .await?
            .pop())
    }

    async fn latest_friend_request(
        &self,
        owner: &DID,
        peer: &DID,
        direction: FriendRequestDirection,
    ) -> Result<Option<FriendRequest>> {
        Ok(self
            .query_friend_requests(
                owner,
                " AND peer_did = ? AND direction = ?",
                vec![peer.to_string(), direction_name(direction).to_string()],
                "ORDER BY updated_at_ms DESC LIMIT 1",
            )
            .await?
            .pop())
    }

    /// The live pending request with `peer`, either direction.
    pub(crate) async fn pending_friend_request(
        &self,
        owner: &DID,
        peer: &DID,
    ) -> Result<Option<FriendRequest>> {
        let now = Self::now_ms();
        Ok(self
            .query_friend_requests(
                owner,
                " AND peer_did = ? AND state = 'pending'",
                vec![peer.to_string()],
                "ORDER BY updated_at_ms DESC",
            )
            .await?
            .into_iter()
            .find(|request| request.expires_at_ms > now))
    }

    async fn save_friend_request(&self, owner: &DID, request: &FriendRequest) -> Result<()> {
        let sql = self.msg_box_db.render_sql(&format!(
            "INSERT INTO friend_requests (owner_key, {FRIEND_COLUMNS}) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(owner_key, peer_did, request_id) DO UPDATE SET \
             state = excluded.state, message = excluded.message, msg_id = excluded.msg_id, \
             peer_name = excluded.peer_name, peer_avatar = excluded.peer_avatar, \
             delivery_json = excluded.delivery_json, expires_at_ms = excluded.expires_at_ms, \
             updated_at_ms = excluded.updated_at_ms"
        ));
        sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(request.request_id.clone())
            .bind(request.peer_did.to_string())
            .bind(direction_name(request.direction).to_string())
            .bind(request.state.as_str().to_string())
            .bind(request.message.clone())
            .bind(request.msg_id.clone())
            .bind(request.peer_name.clone())
            .bind(request.peer_avatar.clone())
            .bind(
                request
                    .delivery
                    .as_ref()
                    .and_then(|delivery| serde_json::to_string(delivery).ok()),
            )
            .bind(request.created_at_ms as i64)
            .bind(request.expires_at_ms as i64)
            .bind(request.updated_at_ms as i64)
            .execute(self.msg_box_db.pool())
            .await
            .map_err(db_error)?;
        Self::publish_friend_request_changed(owner, &request.peer_did, &request.request_id);
        Ok(())
    }

    fn publish_friend_request_changed(owner: &DID, peer: &DID, request_id: &str) {
        let event = ContactChangedEvent {
            owner: owner.clone(),
            operation: "friend_request".into(),
            dids: vec![peer.clone()],
            request_id: Some(request_id.to_string()),
            updated_at_ms: Self::now_ms(),
        };
        Self::publish_event(
            contact_changed_event_id(owner),
            serde_json::to_value(event).unwrap_or_default(),
        );
    }

    async fn set_request_state(
        &self,
        owner: &DID,
        mut request: FriendRequest,
        state: FriendRequestState,
    ) -> Result<FriendRequest> {
        request.state = state;
        request.updated_at_ms = Self::now_ms();
        self.save_friend_request(owner, &request).await?;
        Ok(request)
    }

    /// Both sides now count as friends: the peer may write to the INBOX
    /// unless the owner blocked them.
    async fn become_friends(&self, owner: &DID, peer: &DID) -> Result<()> {
        self.contact_mgr
            .mutate_contact(Some(owner), peer, Some(ContactSource::ManualCreate), |contact| {
                contact.friend_state = FriendState::Friend;
                if contact.access_level != AccessGroupLevel::Block {
                    contact.access_level = AccessGroupLevel::Allow;
                    contact.temp_grants.clear();
                }
            })
            .await?;
        Ok(())
    }

    async fn clear_friend_state(&self, owner: &DID, peer: &DID, from: FriendState) -> Result<()> {
        self.contact_mgr
            .mutate_contact(Some(owner), peer, None, |contact| {
                if contact.friend_state == from {
                    contact.friend_state = FriendState::None;
                }
            })
            .await?;
        Ok(())
    }

    /// Ending a friendship returns the delivery policy to stranger.
    async fn end_friendship(&self, owner: &DID, peer: &DID) -> Result<FriendState> {
        let contact = self
            .contact_mgr
            .mutate_contact(Some(owner), peer, None, |contact| {
                if contact.friend_state == FriendState::Friend {
                    contact.friend_state = FriendState::None;
                    if contact.access_level == AccessGroupLevel::Allow {
                        contact.access_level = AccessGroupLevel::Stranger;
                    }
                }
            })
            .await?;
        Ok(contact.map(|c| c.friend_state).unwrap_or_default())
    }

    async fn own_profile_hint(&self, owner: &DID) -> FriendProfileHint {
        let own = self
            .contact_mgr
            .get_contact(owner.clone(), Some(owner.clone()))
            .await
            .ok()
            .flatten();
        FriendProfileHint {
            name: own
                .as_ref()
                .map(|c| c.name.trim().to_string())
                .filter(|name| !name.is_empty() && name != &owner.to_string()),
            avatar: own
                .and_then(|c| c.avatar)
                .filter(|avatar| avatar.len() <= MAX_PROFILE_HINT_AVATAR_CHARS),
        }
    }

    fn friend_message(owner: &DID, outgoing: &Outgoing) -> Result<MsgObject> {
        Ok(MsgObject {
            from: owner.clone(),
            to: vec![outgoing.peer.clone()],
            kind: MsgObjKind::Operation,
            created_at_ms: Self::now_ms(),
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: outgoing.text.clone(),
                machine: Some(MachineContent {
                    intent: Some(FRIEND_INTENT.into()),
                    data: serde_json::from_value(json!(outgoing.data)).map_err(db_error)?,
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    /// Store the message and hand it to the peer: dispatch directly for a
    /// local peer, a native DeliveryRecord otherwise. A request keeps a SENT
    /// record so the owner's conversation shows it.
    fn send_friend_operation<'a>(
        &'a self,
        owner: &'a DID,
        outgoing: Outgoing,
    ) -> Pin<Box<dyn Future<Output = Result<(String, FriendDelivery)>> + Send + 'a>> {
        Box::pin(async move {
            let msg = Self::friend_message(owner, &outgoing)?;
            let (msg_id, msg_json) = msg.gen_obj_id();
            self.store_message(&msg_id, &msg_json, &msg, crate::buckets::Arrival::Local)
                .await?;
            self.with_state_write(|state| {
                Self::ensure_message(state, msg.clone());
                Ok(())
            })?;
            let peer = outgoing.peer.clone();
            let sent = if outgoing.keep_sent {
                Some(Self::build_mailbox_record(
                    owner.clone(),
                    MailboxKind::Sent,
                    &msg,
                    RecipientState::Read,
                    None,
                    Vec::new(),
                    "owner-sent",
                )?)
            } else {
                None
            };
            let now = Self::now_ms();
            let local = self.is_local_recipient(&peer);
            let mut deliveries = Vec::new();
            let mut planned = Ok(());
            if !local {
                match self.build_delivery_envelope(&msg_id, peer.clone()) {
                    Ok(envelope) => deliveries.push(DeliveryRecord {
                        delivery_id: Self::build_delivery_id(
                            &msg_id,
                            &peer,
                            &envelope.transport_did,
                        ),
                        envelope,
                        state: DeliveryState::Wait,
                        attempts: 0,
                        next_retry_at_ms: None,
                        external_msg_id: None,
                        delivered_at_ms: None,
                        last_error: None,
                        created_at_ms: now,
                        updated_at_ms: now,
                    }),
                    Err(reason) => planned = Err(reason),
                }
            }
            if sent.is_some() || !deliveries.is_empty() {
                let result = PostSendResult {
                    ok: true,
                    msg_id: msg_id.clone(),
                    deliveries: Vec::new(),
                    reason: None,
                };
                self.msg_box_db
                    .commit_post_send_records(
                        "friend",
                        &owner.to_string(),
                        None,
                        None,
                        &msg_id,
                        &msg,
                        sent.as_ref(),
                        &deliveries,
                        &result,
                        now,
                        None,
                    )
                    .await?;
                if let Some(sent) = sent.as_ref() {
                    self.note_records_committed(std::slice::from_ref(sent)).await;
                    Self::publish_box_changed_event(sent, "upsert");
                }
                for record in &deliveries {
                    Self::publish_delivery_changed_event(record, "enqueue");
                }
            }
            let delivery = if local {
                let target = format!("https://local/{}/inbox", peer.to_string());
                match self.dispatch_to_receiver(msg, None, peer.clone(), target).await {
                    Ok(result) if result.delivered_recipients.contains(&peer) => FriendDelivery {
                        state: FriendDeliveryState::Delivered,
                        reason: None,
                    },
                    Ok(_) => FriendDelivery {
                        state: FriendDeliveryState::Failed,
                        reason: Some("rejected".into()),
                    },
                    Err(error) => {
                        warn!("local friend delivery to {} failed: {}", peer.to_string(), error);
                        FriendDelivery {
                            state: FriendDeliveryState::Failed,
                            reason: Some(error.to_string()),
                        }
                    }
                }
            } else {
                match planned {
                    Ok(()) => FriendDelivery {
                        state: FriendDeliveryState::Pending,
                        reason: None,
                    },
                    Err(reason) => FriendDelivery {
                        state: FriendDeliveryState::Failed,
                        reason: Some(reason),
                    },
                }
            };
            Ok((msg_id.to_string(), delivery))
        })
    }

    async fn send_all(&self, owner: &DID, outgoing: Vec<Outgoing>) {
        for message in outgoing {
            let peer = message.peer.clone();
            let action = message.data.action.clone();
            if let Err(error) = self.send_friend_operation(owner, message).await {
                warn!(
                    "send friend {} from {} to {} failed: {}",
                    action,
                    owner.to_string(),
                    peer.to_string(),
                    error
                );
            }
        }
    }

    /// Outgoing requests queued for another Zone report the native delivery.
    async fn with_delivery(&self, mut request: FriendRequest) -> FriendRequest {
        if request.direction != FriendRequestDirection::Outgoing
            || request
                .delivery
                .as_ref()
                .is_some_and(|d| d.state != FriendDeliveryState::Pending)
        {
            return request;
        }
        let Some(msg_id) = request
            .msg_id
            .as_deref()
            .and_then(|id| ndn_lib::ObjId::new(id).ok())
        else {
            return request;
        };
        let Ok(records) = self.msg_box_db.list_deliveries_for_msg(&msg_id).await else {
            return request;
        };
        if let Some(record) = records.into_iter().next() {
            request.delivery = Some(match record.state {
                DeliveryState::Sent => FriendDelivery {
                    state: FriendDeliveryState::Delivered,
                    reason: None,
                },
                DeliveryState::Dead | DeliveryState::Failed => FriendDelivery {
                    state: FriendDeliveryState::Failed,
                    reason: record.last_error.map(|error| error.message),
                },
                _ => FriendDelivery {
                    state: FriendDeliveryState::Pending,
                    reason: record.last_error.map(|error| error.message),
                },
            });
        }
        request
    }

    /// Sending a pending request again revives its native delivery when that
    /// gave up: the same message is queued anew.
    async fn requeue_dead_delivery(&self, request: &FriendRequest) -> Result<()> {
        let Some(msg_id) = request
            .msg_id
            .as_deref()
            .and_then(|id| ndn_lib::ObjId::new(id).ok())
        else {
            return Ok(());
        };
        let mut requeued = false;
        for mut record in self.msg_box_db.list_deliveries_for_msg(&msg_id).await? {
            if !matches!(record.state, DeliveryState::Dead | DeliveryState::Failed) {
                continue;
            }
            record.state = DeliveryState::Wait;
            record.attempts = 0;
            record.next_retry_at_ms = None;
            record.updated_at_ms = Self::now_ms();
            self.msg_box_db.upsert_delivery(&record).await?;
            Self::publish_delivery_changed_event(&record, "enqueue");
            requeued = true;
        }
        if requeued {
            info!(
                "friend request {} to {} queued for delivery again",
                request.request_id,
                request.peer_did.to_string()
            );
            self.publish_sent_delivery_changed(&msg_id).await;
        }
        Ok(())
    }

    async fn canonical_peer(&self, owner: &DID, peer: &DID) -> Result<DID> {
        self.contact_mgr
            .resolve_canonical_did(peer.clone(), Some(owner.clone()))
            .await
    }

    pub(crate) async fn send_friend_request(
        &self,
        owner: &DID,
        peer: &DID,
        message: Option<String>,
    ) -> Result<FriendRequestResult> {
        let peer = self.canonical_peer(owner, peer).await?;
        if &peer == owner {
            return Err(invalid("cannot-befriend-self"));
        }
        if peer.method == "msgtunnel" {
            return Err(invalid("peer-not-befriendable"));
        }
        let message = message
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        if message
            .as_ref()
            .is_some_and(|m| m.chars().count() > FRIEND_REQUEST_MESSAGE_MAX_CHARS)
        {
            return Err(invalid("message-too-long"));
        }
        let guard = self.friend_lock.lock().await;
        let contact = self
            .contact_mgr
            .get_contact(peer.clone(), Some(owner.clone()))
            .await?;
        if contact
            .as_ref()
            .is_some_and(|c| c.access_level == AccessGroupLevel::Block)
        {
            return Err(denied("peer-blocked"));
        }
        if contact
            .as_ref()
            .is_some_and(|c| c.friend_state == FriendState::Friend)
        {
            return Ok(FriendRequestResult {
                friend_state: FriendState::Friend,
                request: None,
            });
        }
        let now = Self::now_ms();
        if let Some(pending) = self.pending_friend_request(owner, &peer).await? {
            if pending.direction == FriendRequestDirection::Incoming {
                drop(guard);
                return self
                    .respond_friend_request(owner, &pending.request_id, FriendRequestDecision::Accept)
                    .await;
            }
            self.requeue_dead_delivery(&pending).await?;
            return Ok(FriendRequestResult {
                friend_state: FriendState::OutgoingPending,
                request: Some(self.with_delivery(pending).await),
            });
        }
        if let Some(last) = self
            .latest_friend_request(owner, &peer, FriendRequestDirection::Outgoing)
            .await?
        {
            if last.state == FriendRequestState::Rejected
                && last.updated_at_ms.saturating_add(FRIEND_REQUEST_COOLDOWN_MS) > now
            {
                return Err(denied("friend-request-cooldown"));
            }
        }
        let request_id = uuid::Uuid::new_v4().simple().to_string();
        let expires_at_ms = now.saturating_add(FRIEND_REQUEST_TTL_MS);
        self.contact_mgr
            .mutate_contact(Some(owner), &peer, Some(ContactSource::ManualCreate), |contact| {
                contact.friend_state = FriendState::OutgoingPending;
            })
            .await?;
        let mut request = FriendRequest {
            request_id: request_id.clone(),
            peer_did: peer.clone(),
            direction: FriendRequestDirection::Outgoing,
            state: FriendRequestState::Pending,
            message: message.clone(),
            msg_id: None,
            peer_name: contact
                .as_ref()
                .map(|c| c.name.clone())
                .filter(|name| !name.is_empty() && name != &peer.to_string()),
            peer_avatar: contact.as_ref().and_then(|c| c.avatar.clone()),
            delivery: None,
            created_at_ms: now,
            expires_at_ms,
            updated_at_ms: now,
        };
        self.save_friend_request(owner, &request).await?;
        drop(guard);

        let outgoing = Outgoing {
            peer: peer.clone(),
            data: FriendMessageData {
                action: FRIEND_ACTION_REQUEST.into(),
                request_id: request_id.clone(),
                message: message.clone(),
                profile_hint: Some(self.own_profile_hint(owner).await),
                expires_at_ms: Some(expires_at_ms),
            },
            text: message.unwrap_or_else(|| "Friend request".into()),
            keep_sent: true,
        };
        let (msg_id, delivery) = self.send_friend_operation(owner, outgoing).await?;
        info!(
            "friend request sent: owner={} peer={} request_id={} delivery={:?}",
            owner.to_string(),
            peer.to_string(),
            request_id,
            delivery.state
        );
        // The peer may already have answered (cross request, local peer).
        let latest = self
            .friend_request_with(owner, &peer, &request_id)
            .await?
            .unwrap_or(request.clone());
        request = latest;
        request.msg_id = Some(msg_id);
        request.delivery = Some(delivery);
        request.updated_at_ms = Self::now_ms();
        self.save_friend_request(owner, &request).await?;
        let friend_state = self.friend_state_of(owner, &peer).await?;
        Ok(FriendRequestResult {
            friend_state,
            request: Some(self.with_delivery(request).await),
        })
    }

    async fn friend_state_of(&self, owner: &DID, peer: &DID) -> Result<FriendState> {
        Ok(self
            .contact_mgr
            .get_contact(peer.clone(), Some(owner.clone()))
            .await?
            .map(|c| c.friend_state)
            .unwrap_or_default())
    }

    async fn live_request(
        &self,
        owner: &DID,
        request_id: &str,
        direction: FriendRequestDirection,
    ) -> Result<FriendRequest> {
        let request = self
            .friend_request_by_id(owner, request_id, direction)
            .await?
            .ok_or_else(|| denied("not-found"))?;
        if request.state != FriendRequestState::Pending {
            return Err(invalid("request-not-pending"));
        }
        if request.expires_at_ms <= Self::now_ms() {
            self.expire_request(owner, request).await?;
            return Err(invalid("request-expired"));
        }
        Ok(request)
    }

    pub(crate) async fn respond_friend_request(
        &self,
        owner: &DID,
        request_id: &str,
        decision: FriendRequestDecision,
    ) -> Result<FriendRequestResult> {
        let guard = self.friend_lock.lock().await;
        let request = self
            .live_request(owner, request_id, FriendRequestDirection::Incoming)
            .await?;
        let peer = request.peer_did.clone();
        let mut outgoing = Vec::new();
        let request = match decision {
            FriendRequestDecision::Accept => {
                self.become_friends(owner, &peer).await?;
                if let Some(mine) = self
                    .latest_friend_request(owner, &peer, FriendRequestDirection::Outgoing)
                    .await?
                    .filter(|r| r.state == FriendRequestState::Pending)
                {
                    self.set_request_state(owner, mine, FriendRequestState::Accepted)
                        .await?;
                }
                outgoing.push(Outgoing::control(
                    &peer,
                    FRIEND_ACTION_ACCEPT,
                    request_id,
                    "Friend request accepted",
                ));
                self.set_request_state(owner, request, FriendRequestState::Accepted)
                    .await?
            }
            FriendRequestDecision::Reject => {
                self.clear_friend_state(owner, &peer, FriendState::IncomingPending)
                    .await?;
                outgoing.push(Outgoing::control(
                    &peer,
                    FRIEND_ACTION_REJECT,
                    request_id,
                    "Friend request declined",
                ));
                self.set_request_state(owner, request, FriendRequestState::Rejected)
                    .await?
            }
            FriendRequestDecision::Ignore => {
                self.clear_friend_state(owner, &peer, FriendState::IncomingPending)
                    .await?;
                self.set_request_state(owner, request, FriendRequestState::Ignored)
                    .await?
            }
        };
        drop(guard);
        self.send_all(owner, outgoing).await;
        Ok(FriendRequestResult {
            friend_state: self.friend_state_of(owner, &peer).await?,
            request: Some(request),
        })
    }

    pub(crate) async fn cancel_friend_request(
        &self,
        owner: &DID,
        request_id: &str,
    ) -> Result<FriendRequestResult> {
        let guard = self.friend_lock.lock().await;
        let request = self
            .live_request(owner, request_id, FriendRequestDirection::Outgoing)
            .await?;
        let peer = request.peer_did.clone();
        self.clear_friend_state(owner, &peer, FriendState::OutgoingPending)
            .await?;
        let request = self
            .set_request_state(owner, request, FriendRequestState::Cancelled)
            .await?;
        drop(guard);
        self.send_all(
            owner,
            vec![Outgoing::control(
                &peer,
                FRIEND_ACTION_CANCEL,
                request_id,
                "Friend request withdrawn",
            )],
        )
        .await;
        Ok(FriendRequestResult {
            friend_state: self.friend_state_of(owner, &peer).await?,
            request: Some(request),
        })
    }

    pub(crate) async fn remove_friend(
        &self,
        owner: &DID,
        peer: &DID,
        notify: bool,
    ) -> Result<FriendRequestResult> {
        let peer = self.canonical_peer(owner, peer).await?;
        let guard = self.friend_lock.lock().await;
        let contact = self
            .contact_mgr
            .get_contact(peer.clone(), Some(owner.clone()))
            .await?
            .ok_or_else(|| denied("not-found"))?;
        if crate::contact_mgr::ContactMgr::is_system_relation(&contact) {
            return Err(denied("system-relation"));
        }
        let was_friend = contact.friend_state == FriendState::Friend;
        let friend_state = self.end_friendship(owner, &peer).await?;
        drop(guard);
        if was_friend && notify {
            self.send_all(
                owner,
                vec![Outgoing::control(&peer, FRIEND_ACTION_REMOVE, "", "Removed from friends")],
            )
            .await;
        }
        Ok(FriendRequestResult {
            friend_state,
            request: None,
        })
    }

    /// Settle the relationship before a contact record is deleted or the
    /// peer blocked: the owner's pending request is withdrawn, the peer's is
    /// ignored, and with `end` the friendship ends too (the peer is told).
    pub(crate) async fn settle_friendship(&self, owner: &DID, peer: &DID, end: bool) -> Result<()> {
        let peer = &self.canonical_peer(owner, peer).await?;
        let guard = self.friend_lock.lock().await;
        let mut outgoing = Vec::new();
        if let Some(contact) = self
            .contact_mgr
            .get_contact(peer.clone(), Some(owner.clone()))
            .await?
            .filter(|_| end)
        {
            if contact.friend_state == FriendState::Friend
                && !crate::contact_mgr::ContactMgr::is_system_relation(&contact)
            {
                self.end_friendship(owner, peer).await?;
                outgoing.push(Outgoing::control(peer, FRIEND_ACTION_REMOVE, "", "Removed from friends"));
            }
        }
        if let Some(pending) = self.pending_friend_request(owner, peer).await? {
            match pending.direction {
                FriendRequestDirection::Outgoing => {
                    self.clear_friend_state(owner, peer, FriendState::OutgoingPending)
                        .await?;
                    outgoing.push(Outgoing::control(
                        peer,
                        FRIEND_ACTION_CANCEL,
                        &pending.request_id,
                        "Friend request withdrawn",
                    ));
                    self.set_request_state(owner, pending, FriendRequestState::Cancelled)
                        .await?;
                }
                FriendRequestDirection::Incoming => {
                    self.clear_friend_state(owner, peer, FriendState::IncomingPending)
                        .await?;
                    self.set_request_state(owner, pending, FriendRequestState::Ignored)
                        .await?;
                }
            }
        }
        drop(guard);
        self.send_all(owner, outgoing).await;
        Ok(())
    }

    pub(crate) async fn list_friend_requests(
        &self,
        owner: &DID,
        req: &ContactListFriendRequestsReq,
    ) -> Result<FriendRequestPage> {
        self.expire_friend_requests_of(owner).await?;
        let mut clause = String::new();
        let mut binds = Vec::new();
        if let Some(direction) = req.direction {
            clause.push_str(" AND direction = ?");
            binds.push(direction_name(direction).to_string());
        }
        if let Some(state) = req.state {
            clause.push_str(" AND state = ?");
            binds.push(state.as_str().to_string());
        }
        let limit = req
            .limit
            .unwrap_or(DEFAULT_FRIEND_REQUEST_LIMIT)
            .clamp(1, MAX_FRIEND_REQUEST_LIMIT);
        let offset = req.offset.unwrap_or(0);
        let rows = self
            .query_friend_requests(
                owner,
                &clause,
                binds,
                &format!("ORDER BY updated_at_ms DESC, request_id ASC LIMIT {limit} OFFSET {offset}"),
            )
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            items.push(self.with_delivery(row).await);
        }
        Ok(FriendRequestPage { items })
    }

    async fn expire_request(&self, owner: &DID, request: FriendRequest) -> Result<()> {
        let peer = request.peer_did.clone();
        let from = match request.direction {
            FriendRequestDirection::Incoming => FriendState::IncomingPending,
            FriendRequestDirection::Outgoing => FriendState::OutgoingPending,
        };
        self.set_request_state(owner, request, FriendRequestState::Expired)
            .await?;
        self.clear_friend_state(owner, &peer, from).await
    }

    async fn expire_friend_requests_of(&self, owner: &DID) -> Result<()> {
        let sql = self.msg_box_db.render_sql(&format!(
            "SELECT {FRIEND_COLUMNS} FROM friend_requests \
             WHERE owner_key = ? AND state = 'pending' AND expires_at_ms <= ?"
        ));
        let rows = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(Self::now_ms() as i64)
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(db_error)?;
        for row in rows.iter() {
            self.expire_request(owner, row_to_request(row)?).await?;
        }
        Ok(())
    }

    /// Periodic sweep over every owner.
    pub(crate) async fn sweep_friend_requests(&self) -> Result<()> {
        let sql = self.msg_box_db.render_sql(
            "SELECT DISTINCT owner_key FROM friend_requests WHERE state = 'pending' AND expires_at_ms <= ?",
        );
        let owners = sqlx::query(&sql)
            .bind(Self::now_ms() as i64)
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(db_error)?;
        for row in owners {
            let owner: String = row.try_get("owner_key").map_err(db_error)?;
            if let Ok(owner) = DID::from_str(&owner) {
                let _guard = self.friend_lock.lock().await;
                self.expire_friend_requests_of(&owner).await?;
            }
        }
        Ok(())
    }

    pub(crate) fn start_friend_request_sweep(&self) {
        let center = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                if let Err(error) = center.sweep_friend_requests().await {
                    warn!("friend request sweep failed: {}", error);
                }
            }
        });
    }

    /// A `buckyos.friend` message from `msg.from` reached `recipient`.
    pub(crate) async fn accept_friend_message(
        &self,
        recipient: &DID,
        msg: &MsgObject,
        data: FriendMessageData,
    ) -> Result<FriendIngress> {
        let peer = self.canonical_peer(recipient, &msg.from).await?;
        if &peer == recipient {
            return Ok(FriendIngress::Drop);
        }
        let guard = self.friend_lock.lock().await;
        let contact = self
            .contact_mgr
            .get_contact(peer.clone(), Some(recipient.clone()))
            .await?;
        if contact
            .as_ref()
            .is_some_and(|c| c.access_level == AccessGroupLevel::Block)
        {
            return Ok(FriendIngress::Drop);
        }
        let now = Self::now_ms();
        let request_id = data.request_id.clone();
        let mut outgoing = Vec::new();
        let ingress = match data.action.as_str() {
            FRIEND_ACTION_REQUEST => {
                if request_id.is_empty() {
                    FriendIngress::Drop
                } else if let Some(existing) =
                    self.friend_request_with(recipient, &peer, &request_id).await?
                {
                    if existing.state == FriendRequestState::Pending {
                        FriendIngress::Mailbox
                    } else {
                        FriendIngress::Control
                    }
                } else if data.expires_at_ms.is_some_and(|t| t <= now) {
                    FriendIngress::Drop
                } else if contact
                    .as_ref()
                    .is_some_and(|c| c.friend_state == FriendState::Friend)
                {
                    outgoing.push(Outgoing::control(
                        &peer,
                        FRIEND_ACTION_ACCEPT,
                        &request_id,
                        "Friend request accepted",
                    ));
                    FriendIngress::Control
                } else if let Some(mine) = self
                    .latest_friend_request(recipient, &peer, FriendRequestDirection::Outgoing)
                    .await?
                    .filter(|r| r.state == FriendRequestState::Pending && r.expires_at_ms > now)
                {
                    // Both asked: the requests answer each other.
                    self.become_friends(recipient, &peer).await?;
                    self.set_request_state(recipient, mine, FriendRequestState::Accepted)
                        .await?;
                    self.save_friend_request(
                        recipient,
                        &FriendRequest {
                            request_id: request_id.clone(),
                            peer_did: peer.clone(),
                            direction: FriendRequestDirection::Incoming,
                            state: FriendRequestState::Accepted,
                            message: data.message.clone(),
                            msg_id: Some(msg.gen_obj_id().0.to_string()),
                            peer_name: data.profile_hint.as_ref().and_then(|h| h.name.clone()),
                            peer_avatar: data.profile_hint.as_ref().and_then(|h| h.avatar.clone()),
                            delivery: None,
                            created_at_ms: now,
                            expires_at_ms: data.expires_at_ms.unwrap_or(now + FRIEND_REQUEST_TTL_MS),
                            updated_at_ms: now,
                        },
                    )
                    .await?;
                    outgoing.push(Outgoing::control(
                        &peer,
                        FRIEND_ACTION_ACCEPT,
                        &request_id,
                        "Friend request accepted",
                    ));
                    FriendIngress::Control
                } else if self
                    .latest_friend_request(recipient, &peer, FriendRequestDirection::Incoming)
                    .await?
                    .is_some_and(|last| {
                        matches!(last.state, FriendRequestState::Rejected | FriendRequestState::Ignored)
                            && last.updated_at_ms.saturating_add(FRIEND_REQUEST_COOLDOWN_MS) > now
                    })
                {
                    FriendIngress::Drop
                } else {
                    let hint = data.profile_hint.clone().unwrap_or_default();
                    self.contact_mgr
                        .mutate_contact(
                            Some(recipient),
                            &peer,
                            Some(ContactSource::AutoInferred),
                            |contact| {
                                if contact.friend_state != FriendState::Friend {
                                    contact.friend_state = FriendState::IncomingPending;
                                }
                                if contact.name.trim().is_empty() || contact.name == contact.did.to_string() {
                                    if let Some(name) = hint.name.clone() {
                                        contact.name = name;
                                    }
                                }
                                if contact.avatar.is_none() {
                                    contact.avatar = hint.avatar.clone();
                                }
                            },
                        )
                        .await?;
                    self.save_friend_request(
                        recipient,
                        &FriendRequest {
                            request_id: request_id.clone(),
                            peer_did: peer.clone(),
                            direction: FriendRequestDirection::Incoming,
                            state: FriendRequestState::Pending,
                            message: data
                                .message
                                .clone()
                                .map(|m| m.chars().take(FRIEND_REQUEST_MESSAGE_MAX_CHARS).collect()),
                            msg_id: Some(msg.gen_obj_id().0.to_string()),
                            peer_name: hint.name,
                            peer_avatar: hint.avatar,
                            delivery: None,
                            created_at_ms: now,
                            expires_at_ms: data
                                .expires_at_ms
                                .unwrap_or(now + FRIEND_REQUEST_TTL_MS)
                                .min(now + FRIEND_REQUEST_TTL_MS),
                            updated_at_ms: now,
                        },
                    )
                    .await?;
                    FriendIngress::Mailbox
                }
            }
            FRIEND_ACTION_ACCEPT => {
                if let Some(mine) = self
                    .friend_request_with(recipient, &peer, &request_id)
                    .await?
                    .filter(|r| {
                        r.direction == FriendRequestDirection::Outgoing
                            && matches!(r.state, FriendRequestState::Pending | FriendRequestState::Accepted)
                    })
                {
                    if mine.state == FriendRequestState::Pending {
                        self.become_friends(recipient, &peer).await?;
                        self.set_request_state(recipient, mine, FriendRequestState::Accepted)
                            .await?;
                    }
                }
                FriendIngress::Control
            }
            FRIEND_ACTION_REJECT => {
                if let Some(mine) = self
                    .friend_request_with(recipient, &peer, &request_id)
                    .await?
                    .filter(|r| {
                        r.direction == FriendRequestDirection::Outgoing
                            && r.state == FriendRequestState::Pending
                    })
                {
                    self.clear_friend_state(recipient, &peer, FriendState::OutgoingPending)
                        .await?;
                    self.set_request_state(recipient, mine, FriendRequestState::Rejected)
                        .await?;
                }
                FriendIngress::Control
            }
            FRIEND_ACTION_CANCEL => {
                if let Some(theirs) = self
                    .friend_request_with(recipient, &peer, &request_id)
                    .await?
                    .filter(|r| {
                        r.direction == FriendRequestDirection::Incoming
                            && r.state == FriendRequestState::Pending
                    })
                {
                    self.clear_friend_state(recipient, &peer, FriendState::IncomingPending)
                        .await?;
                    self.set_request_state(recipient, theirs, FriendRequestState::Cancelled)
                        .await?;
                }
                FriendIngress::Control
            }
            FRIEND_ACTION_REMOVE => {
                if contact
                    .as_ref()
                    .is_some_and(|c| !crate::contact_mgr::ContactMgr::is_system_relation(c))
                {
                    self.end_friendship(recipient, &peer).await?;
                }
                FriendIngress::Control
            }
            _ => FriendIngress::Drop,
        };
        drop(guard);
        self.send_all(recipient, outgoing).await;
        Ok(ingress)
    }
}
