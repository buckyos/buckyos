//! Relationship surface of the Contact Mgr in msg-center (My Network): lookup,
//! friend state and friend requests, contact tags and their write rules.
//!
//! A friend request is a `kind = Operation` MsgObject with machine intent
//! [`FRIEND_INTENT`]; msg-center builds it for the user and delivers it on the
//! ordinary native path. Each side keeps its own [`FriendRequest`] records and
//! the [`FriendState`] of the contact; the two sides are not required to agree
//! at every moment.
//!
//! Tags group contacts. A tag is a system tag ([`CONTACT_TAG_ZONE_USER`],
//! [`CONTACT_TAG_ZONE_AGENT`], or a user group reference `type:<user type>` /
//! `group:<id>` naming the source group of a projected Zone user), an app tag
//! (`<app>.<name>`, written only by that app) or a user tag (anything else).

use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FRIEND_INTENT: &str = "buckyos.friend";
pub const FRIEND_ACTION_REQUEST: &str = "request";
pub const FRIEND_ACTION_ACCEPT: &str = "accept";
pub const FRIEND_ACTION_REJECT: &str = "reject";
pub const FRIEND_ACTION_CANCEL: &str = "cancel";
pub const FRIEND_ACTION_REMOVE: &str = "remove";
pub const FRIEND_REQUEST_TTL_MS: u64 = 7 * 24 * 3600 * 1000;
/// After a request from a peer was rejected or ignored, further requests from
/// that peer are dropped for this long.
pub const FRIEND_REQUEST_COOLDOWN_MS: u64 = 24 * 3600 * 1000;
pub const FRIEND_REQUEST_MESSAGE_MAX_CHARS: usize = 500;

pub const CONTACT_TAG_ZONE_USER: &str = "zone_user";
pub const CONTACT_TAG_ZONE_AGENT: &str = "zone_agent";
pub const CONTACT_TAG_HOMESTATION_FOLLOWING: &str = "homestation.following";
pub const CONTACT_TAG_MAX_CHARS: usize = 64;
/// RBAC resource a service needs (`write`) to change the tags of its namespace:
/// `obj://msg-center/contact_tags/<app>`.
pub const CONTACT_TAGS_RBAC_RESOURCE_PREFIX: &str = "obj://msg-center/contact_tags/";
/// Name of the kevent published under `/msg_center/<owner>/CONTACTS/`.
pub const CONTACT_CHANGED_EVENT_BOX: &str = "CONTACTS";

pub const METHOD_CONTACT_LOOKUP: &str = "contact.lookup";
pub const METHOD_CONTACT_UPDATE_TAGS: &str = "contact.update_tags";
pub const METHOD_CONTACT_REMOVE_CONTACT: &str = "contact.remove_contact";
pub const METHOD_CONTACT_UNBLOCK_CONTACT: &str = "contact.unblock_contact";
pub const METHOD_CONTACT_SEND_FRIEND_REQUEST: &str = "contact.send_friend_request";
pub const METHOD_CONTACT_RESPOND_FRIEND_REQUEST: &str = "contact.respond_friend_request";
pub const METHOD_CONTACT_CANCEL_FRIEND_REQUEST: &str = "contact.cancel_friend_request";
pub const METHOD_CONTACT_REMOVE_FRIEND: &str = "contact.remove_friend";
pub const METHOD_CONTACT_LIST_FRIEND_REQUESTS: &str = "contact.list_friend_requests";
/// Service-only: is a DID in one of an owner's contact groups (T-5).
pub const METHOD_CONTACT_CHECK_MEMBERSHIP: &str = "contact.check_membership";
/// RBAC resource a service needs (`read`) for [`METHOD_CONTACT_CHECK_MEMBERSHIP`].
pub const CONTACT_MEMBERSHIP_RBAC_RESOURCE: &str = "obj://msg-center/contact_membership";

pub const CONTACT_EXT_METHODS: [&str; 10] = [
    METHOD_CONTACT_LOOKUP,
    METHOD_CONTACT_UPDATE_TAGS,
    METHOD_CONTACT_REMOVE_CONTACT,
    METHOD_CONTACT_UNBLOCK_CONTACT,
    METHOD_CONTACT_SEND_FRIEND_REQUEST,
    METHOD_CONTACT_RESPOND_FRIEND_REQUEST,
    METHOD_CONTACT_CANCEL_FRIEND_REQUEST,
    METHOD_CONTACT_REMOVE_FRIEND,
    METHOD_CONTACT_LIST_FRIEND_REQUESTS,
    METHOD_CONTACT_CHECK_MEMBERSHIP,
];

/// The friendship between the owner and a contact, as the owner's side knows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendState {
    #[default]
    None,
    OutgoingPending,
    IncomingPending,
    Friend,
}

impl FriendState {
    pub fn is_none(&self) -> bool {
        *self == Self::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactTagKind {
    System,
    App,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WellKnownContactTag {
    pub key: &'static str,
    pub kind: ContactTagKind,
    /// App that owns the tag; `None` for system tags.
    pub app: Option<&'static str>,
    /// Whether the user may add or remove members.
    pub user_editable: bool,
}

pub const WELL_KNOWN_CONTACT_TAGS: [WellKnownContactTag; 3] = [
    WellKnownContactTag {
        key: CONTACT_TAG_ZONE_USER,
        kind: ContactTagKind::System,
        app: None,
        user_editable: false,
    },
    WellKnownContactTag {
        key: CONTACT_TAG_ZONE_AGENT,
        kind: ContactTagKind::System,
        app: None,
        user_editable: false,
    },
    WellKnownContactTag {
        key: CONTACT_TAG_HOMESTATION_FOLLOWING,
        kind: ContactTagKind::App,
        app: Some("homestation"),
        user_editable: false,
    },
];

/// `zone_user` / `zone_agent` are system tags; `<app>.<name>` with a
/// lowercase ASCII `app` is an app tag; anything else is a user tag.
pub fn contact_tag_kind(tag: &str) -> ContactTagKind {
    if tag == CONTACT_TAG_ZONE_USER
        || tag == CONTACT_TAG_ZONE_AGENT
        || tag.starts_with(crate::USER_GROUP_REF_PREFIX)
        || tag.starts_with(crate::USER_TYPE_GROUP_REF_PREFIX)
    {
        return ContactTagKind::System;
    }
    if contact_tag_app(tag).is_some() {
        return ContactTagKind::App;
    }
    ContactTagKind::User
}

/// The app namespace of an app tag.
pub fn contact_tag_app(tag: &str) -> Option<&str> {
    let (app, rest) = tag.split_once('.')?;
    let ident = |s: &str, extra: char| {
        s.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == extra
        })
    };
    let valid = !app.is_empty()
        && !rest.is_empty()
        && app.starts_with(|c: char| c.is_ascii_lowercase())
        && ident(app, '-')
        && ident(rest, '.');
    valid.then_some(app)
}

/// A tag value as stored: trimmed, non-empty, bounded.
pub fn normalize_contact_tag(raw: &str) -> Option<String> {
    let tag = raw.trim();
    (!tag.is_empty() && tag.chars().count() <= CONTACT_TAG_MAX_CHARS && !tag.contains('\n'))
        .then(|| tag.to_string())
}

/// `machine.data` of a [`FRIEND_INTENT`] message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendMessageData {
    pub action: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_hint: Option<FriendProfileHint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendProfileHint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestDirection {
    Incoming,
    Outgoing,
}

impl FriendRequestDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestState {
    Pending,
    Accepted,
    Rejected,
    Cancelled,
    Expired,
    Ignored,
}

impl FriendRequestState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Ignored => "ignored",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "pending" => Self::Pending,
            "accepted" => Self::Accepted,
            "rejected" => Self::Rejected,
            "cancelled" => Self::Cancelled,
            "expired" => Self::Expired,
            "ignored" => Self::Ignored,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestDecision {
    Accept,
    Reject,
    Ignore,
}

/// How far the message of an outgoing request got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendDeliveryState {
    /// Queued for another Zone; not confirmed yet.
    Pending,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendDelivery {
    pub state: FriendDeliveryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequest {
    pub request_id: String,
    pub peer_did: DID,
    pub direction: FriendRequestDirection,
    pub state: FriendRequestState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_avatar: Option<String>,
    /// Outgoing requests only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<FriendDelivery>,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequestResult {
    pub friend_state: FriendState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<FriendRequest>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequestPage {
    #[serde(default)]
    pub items: Vec<FriendRequest>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactUpdateTagsReq {
    pub did: DID,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

/// Exact match on the authenticated `did` (no alias / merge resolution);
/// projected Zone users count with their source group tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactCheckMembershipReq {
    pub owner: DID,
    pub did: DID,
    pub tag: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactCheckMembershipResult {
    pub member: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactDidReq {
    pub did: DID,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactSendFriendRequestReq {
    pub did: DID,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactRespondFriendRequestReq {
    pub request_id: String,
    pub decision: FriendRequestDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactCancelFriendRequestReq {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactRemoveFriendReq {
    pub did: DID,
    /// Tell the peer (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactListFriendRequestsReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<FriendRequestDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<FriendRequestState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupKind {
    User,
    Agent,
    Group,
    Zone,
    Device,
    Msgtunnel,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupSection {
    /// The owner's contacts and joined groups.
    Relation,
    /// Users and Agents of this Zone.
    Zone,
    /// The input itself, resolved through the name service.
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupInputKind {
    Empty,
    Text,
    Did,
    Name,
    InviteLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityState {
    /// Hosted by this Zone.
    Local,
    /// Reached through a message tunnel of this Zone.
    Tunnel,
    /// Another Zone that native delivery can route to.
    Routable,
    /// Not checked by this answer.
    Unknown,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reachability {
    pub state: ReachabilityState,
    /// Zone host the DID belongs to, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The owner's relation to a candidate (absent fields mean "none").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupRelation {
    pub is_contact: bool,
    #[serde(default, skip_serializing_if = "FriendState::is_none")]
    pub friend_state: FriendState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_level: Option<crate::AccessGroupLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The pending friend request with this candidate, either direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_request_id: Option<String>,
    /// Group candidates: the owner's member state (`active`, `invited`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_role: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupGroupInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_policy: Option<String>,
    /// Invite token carried by the input link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<String>,
    /// The group is hosted by this Zone (only these can be joined for now).
    pub hosted_here: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<DID>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupCandidate {
    pub did: DID,
    pub kind: LookupKind,
    pub section: LookupSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub reachability: Reachability,
    pub relation: LookupRelation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<LookupGroupInfo>,
}

/// `invalid-input` / `not-found` / `unreachable` / `forbidden`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupError {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupResult {
    pub input: String,
    pub input_kind: LookupInputKind,
    #[serde(default)]
    pub candidates: Vec<LookupCandidate>,
    /// Why the input itself could not be resolved; local matches may still exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<LookupError>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactLookupReq {
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<LookupKind>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

/// Payload of the `/msg_center/<owner>/CONTACTS/changed` kevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactChangedEvent {
    pub owner: DID,
    /// `upsert` / `remove` / `friend_request`.
    pub operation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dids: Vec<DID>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub updated_at_ms: u64,
}

pub fn contact_changed_event_id(owner: &DID) -> String {
    format!(
        "/msg_center/{}/{}/changed",
        owner.to_raw_host_name(),
        CONTACT_CHANGED_EVENT_BOX
    )
}

/// The machine data of a friend message, if `content` carries one.
pub fn friend_message_data(
    machine_intent: Option<&str>,
    data: &Value,
) -> Option<FriendMessageData> {
    if machine_intent != Some(FRIEND_INTENT) {
        return None;
    }
    serde_json::from_value(data.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_kinds_follow_the_namespace_rule() {
        assert_eq!(contact_tag_kind("zone_user"), ContactTagKind::System);
        assert_eq!(contact_tag_kind("zone_agent"), ContactTagKind::System);
        assert_eq!(
            contact_tag_kind(CONTACT_TAG_HOMESTATION_FOLLOWING),
            ContactTagKind::App
        );
        assert_eq!(
            contact_tag_app("homestation.following"),
            Some("homestation")
        );
        for user in ["家人", "Team A", "a.", ".x", "Work.Friends", "v1.2 notes"] {
            assert_eq!(contact_tag_kind(user), ContactTagKind::User, "{user}");
        }
        assert_eq!(normalize_contact_tag("  客户 "), Some("客户".to_string()));
        assert_eq!(normalize_contact_tag("   "), None);
    }

    #[test]
    fn friend_state_defaults_to_none() {
        #[derive(Deserialize)]
        struct Probe {
            #[serde(default)]
            state: FriendState,
        }
        let probe: Probe = serde_json::from_str("{}").unwrap();
        assert_eq!(probe.state, FriendState::None);
        assert_eq!(
            serde_json::to_value(FriendState::OutgoingPending).unwrap(),
            "outgoing_pending"
        );
    }
}
