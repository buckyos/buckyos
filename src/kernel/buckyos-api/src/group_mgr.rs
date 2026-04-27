//! Self-host group data models and request/response types.
//!
//! Implements section 4 of `doc/message_hub/Self-Host-Group.md`. Types here are
//! shared between the msg-center service (authoritative state) and clients
//! (UI / agent / cross-service callers). Anything that may be sent across a
//! Zone boundary or stored as a CYFS NamedObject is serialised via canonical
//! JSON; for that reason every optional field uses `skip_serializing_if` to
//! keep the canonical form stable.

use ::kRPC::RPCErrors;
use name_lib::DID;
use ndn_lib::ObjId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Section 2.1 — DID entity / collection model.
// ---------------------------------------------------------------------------

/// Whether a DID points at a single entity or at a DID collection.
///
/// First-version BuckyOS recognises exactly one collection kind: self-host
/// group, encoded as `DIDCollection`. New collection kinds must not be added
/// here without updating the protocol-level commitment in section 2.1.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DIDEntityKind {
    SingleEntity,
    DIDCollection,
}

impl Default for DIDEntityKind {
    fn default() -> Self {
        Self::SingleEntity
    }
}

/// The `member_kind` field on `GroupMemberRecord`. Caches the resolved entity
/// kind of the member DID; the canonical answer always comes from resolving
/// the DID Document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DIDMemberKind {
    Unknown,
    SingleEntity,
    CollectionEntity,
}

impl Default for DIDMemberKind {
    fn default() -> Self {
        Self::Unknown
    }
}

// ---------------------------------------------------------------------------
// Section 4.1 — GroupDoc and policy enums.
// ---------------------------------------------------------------------------

/// Primary semantic role the group serves. Lets external systems decide how
/// to surface a `group_did` (e.g. a notification group vs a collaboration
/// group) without inspecting the entire policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupPurpose {
    Conversation,
    Notification,
    Collaboration,
    PermissionScope,
    Organization,
    Custom(String),
}

impl Default for GroupPurpose {
    fn default() -> Self {
        Self::Conversation
    }
}

/// Visibility of either the member list or the admin list. `PublicWithProof`
/// requires that each public member be backed by a verifiable two-way proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipVisibility {
    Private,
    MembersOnly,
    PublicWithProof,
}

impl Default for MembershipVisibility {
    fn default() -> Self {
        Self::MembersOnly
    }
}

/// Whether nested group DIDs are allowed as members and, if so, whether
/// recursive expansion may happen automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NestedGroupPolicy {
    Disallow,
    AllowAsOpaqueMember,
    AllowWithRecursiveExpansion,
}

impl Default for NestedGroupPolicy {
    fn default() -> Self {
        Self::AllowAsOpaqueMember
    }
}

/// Where automatic expansion is permitted. Even when expansion is allowed,
/// callers must respect `max_expansion_depth` and cycle detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupExpansionPolicy {
    NoAutoExpansion,
    ExpandForDelivery,
    ExpandForPermissionCheck,
    ExpandForPublicView,
}

impl Default for GroupExpansionPolicy {
    fn default() -> Self {
        Self::NoAutoExpansion
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupCollectionPolicy {
    pub nested_group_policy: NestedGroupPolicy,
    pub expansion_policy: GroupExpansionPolicy,
    pub max_expansion_depth: u8,
    pub reject_cycles: bool,
}

impl Default for GroupCollectionPolicy {
    fn default() -> Self {
        Self {
            nested_group_policy: NestedGroupPolicy::AllowAsOpaqueMember,
            expansion_policy: GroupExpansionPolicy::NoAutoExpansion,
            max_expansion_depth: 4,
            reject_cycles: true,
        }
    }
}

/// Attribution mode for collaborative authorship and revenue routing.
/// Settlement itself is out of scope for this protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupAttributionMode {
    OpaqueGroupDID,
    PublicMembers,
    ExternalContract,
}

impl Default for GroupAttributionMode {
    fn default() -> Self {
        Self::OpaqueGroupDID
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSplitRule {
    pub did: DID,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupAttributionPolicy {
    pub attribution_mode: GroupAttributionMode,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub revenue_split_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub split_hint: Option<Vec<GroupSplitRule>>,
    pub public_attribution: MembershipVisibility,
}

impl Default for GroupAttributionPolicy {
    fn default() -> Self {
        Self {
            attribution_mode: GroupAttributionMode::OpaqueGroupDID,
            revenue_split_ref: None,
            split_hint: None,
            public_attribution: MembershipVisibility::Private,
        }
    }
}

/// Logical CYFS semantic paths exposed by a group. Stored relative to the
/// host Zone/OOD; the full URL is built by clients from the DID Document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupEndpoints {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message_center: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub group_mgr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub inbox_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub subgroup_inbox_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub join_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub submit_member_proof_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub expand_members_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub admin_operation_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinPolicy {
    InviteOnly,
    RequestAndAdminApprove,
}

impl Default for JoinPolicy {
    fn default() -> Self {
        Self::InviteOnly
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PostPolicy {
    AllMembers,
    AdminOnly,
    RoleBased { roles: Vec<GroupRole> },
}

impl Default for PostPolicy {
    fn default() -> Self {
        Self::AllMembers
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryVisibility {
    NewMembersOnly,
    AllHistory,
}

impl Default for HistoryVisibility {
    fn default() -> Self {
        Self::NewMembersOnly
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    Default,
    Forever,
    Days(u32),
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupDeliveryPreference {
    Default,
    NativeOnly,
    TunnelOnly,
    Mute,
}

impl Default for GroupDeliveryPreference {
    fn default() -> Self {
        Self::Default
    }
}

/// MessageHub-side projection of the group DID Document. The DID Document
/// itself is the root of authority; this struct caches the high-frequency
/// profile / policy fields so the UI does not need a DID Document round-trip
/// for every render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupDoc {
    pub obj_type: String,
    pub schema_version: u32,
    pub group_did: DID,
    pub entity_kind: DIDEntityKind,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub did_doc_id: Option<ObjId>,
    pub doc_version: u64,
    pub host_zone: DID,
    pub owner: DID,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub purpose: GroupPurpose,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub profile_version: u64,
    pub policy: GroupPolicy,
    pub collection_policy: GroupCollectionPolicy,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attribution_policy: Option<GroupAttributionPolicy>,
    pub public_membership: MembershipVisibility,
    pub public_admins: MembershipVisibility,
    pub endpoints: GroupEndpoints,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub proof: Option<String>,
}

pub const GROUP_DOC_OBJ_TYPE: &str = "buckyos.group_doc";
pub const GROUP_DOC_SCHEMA_VERSION: u32 = 1;

/// Header-style policy block that lives on the `GroupDoc`. Distinct from
/// `GroupSettings`, which carries the full editable policy state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupPolicy {
    pub join_policy: JoinPolicy,
    pub post_policy: PostPolicy,
    pub history_visibility: HistoryVisibility,
    pub retention_policy: RetentionPolicy,
    pub default_delivery: GroupDeliveryPreference,
}

impl Default for GroupPolicy {
    fn default() -> Self {
        Self {
            join_policy: JoinPolicy::InviteOnly,
            post_policy: PostPolicy::AllMembers,
            history_visibility: HistoryVisibility::NewMembersOnly,
            retention_policy: RetentionPolicy::Default,
            default_delivery: GroupDeliveryPreference::Default,
        }
    }
}

// ---------------------------------------------------------------------------
// Section 4.2 — GroupMemberRecord.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GroupRole {
    Owner,
    Admin,
    Member,
    Guest,
}

impl Default for GroupRole {
    fn default() -> Self {
        Self::Member
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupMemberState {
    Invited,
    PendingMemberSignature,
    PendingAdminApproval,
    Active,
    Muted,
    Left,
    Removed,
    Blocked,
}

impl Default for GroupMemberState {
    fn default() -> Self {
        Self::Invited
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMemberRecord {
    pub group_did: DID,
    pub member_did: DID,
    pub member_kind: DIDMemberKind,
    pub role: GroupRole,
    pub state: GroupMemberState,
    pub joined_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub invited_by: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub approved_by: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub member_proof_id: Option<ObjId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mute_until_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub delivery_preference: Option<GroupDeliveryPreference>,
}

// ---------------------------------------------------------------------------
// Section 4.3 — GroupMemberProof.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupMemberProofScope {
    JoinAsSelf,
    JoinAsCollectionEntity,
}

impl Default for GroupMemberProofScope {
    fn default() -> Self {
        Self::JoinAsSelf
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMemberProof {
    pub obj_type: String,
    pub schema_version: u32,
    pub group_did: DID,
    pub member_did: DID,
    pub member_kind: DIDMemberKind,
    pub role: GroupRole,
    pub proof_scope: GroupMemberProofScope,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub invite_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
    pub nonce: String,
    pub issued_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub expires_at_ms: Option<u64>,
    pub signer: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub member_zone: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reverse_proof_uri: Option<String>,
    pub proof: String,
}

pub const GROUP_MEMBER_PROOF_OBJ_TYPE: &str = "buckyos.group_member_proof";
pub const GROUP_MEMBER_PROOF_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Section 4.4 — GroupSettings.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSettings {
    pub group_did: DID,
    pub join_policy: JoinPolicy,
    pub post_policy: PostPolicy,
    pub history_visibility: HistoryVisibility,
    pub retention_policy: RetentionPolicy,
    pub default_delivery: GroupDeliveryPreference,
    pub collection_policy: GroupCollectionPolicy,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attribution_policy: Option<GroupAttributionPolicy>,
}

impl GroupSettings {
    pub fn defaults_for(group_did: DID) -> Self {
        Self {
            group_did,
            join_policy: JoinPolicy::InviteOnly,
            post_policy: PostPolicy::AllMembers,
            history_visibility: HistoryVisibility::NewMembersOnly,
            retention_policy: RetentionPolicy::Default,
            default_delivery: GroupDeliveryPreference::Default,
            collection_policy: GroupCollectionPolicy::default(),
            attribution_policy: None,
        }
    }

    pub fn to_policy(&self) -> GroupPolicy {
        GroupPolicy {
            join_policy: self.join_policy,
            post_policy: self.post_policy.clone(),
            history_visibility: self.history_visibility,
            retention_policy: self.retention_policy,
            default_delivery: self.default_delivery,
        }
    }
}

// ---------------------------------------------------------------------------
// Section 4.5 — GroupSubgroup.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSubgroup {
    pub group_did: DID,
    pub subgroup_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub member_dids: Vec<DID>,
    pub created_by: DID,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

// ---------------------------------------------------------------------------
// Section 4.6 — GroupEvent.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupEventType {
    GroupCreated,
    MemberInvited,
    MemberJoinRequested,
    MemberProofAccepted,
    MemberApprovalRequested,
    MemberJoined,
    MemberLeft,
    MemberRemoved,
    RoleChanged,
    ProfileUpdated,
    PolicyUpdated,
    CollectionPolicyUpdated,
    AttributionPolicyUpdated,
    NestedGroupAdded,
    NestedGroupExpanded,
    SubgroupCreated,
    SubgroupUpdated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupEvent {
    pub obj_type: String,
    pub schema_version: u32,
    pub event_id: String,
    pub group_did: DID,
    pub actor: DID,
    pub event_type: GroupEventType,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub target: Option<DID>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub detail: BTreeMap<String, String>,
}

pub const GROUP_EVENT_OBJ_TYPE: &str = "buckyos.group_event";
pub const GROUP_EVENT_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Section 4.7 — GroupExpansionSnapshot.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupExpansionPurpose {
    Delivery,
    PermissionCheck,
    PublicView,
    Attribution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedDID {
    pub did: DID,
    pub member_kind: DIDMemberKind,
    pub via_path: Vec<DID>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<GroupRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupExpansionSnapshot {
    pub obj_type: String,
    pub schema_version: u32,
    pub operation_id: String,
    pub root_group_did: DID,
    pub purpose: GroupExpansionPurpose,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub requested_by: Option<DID>,
    pub created_at_ms: u64,
    pub max_depth: u8,
    pub expanded_members: Vec<ExpandedDID>,
    pub opaque_members: Vec<DID>,
    pub visited_groups: Vec<DID>,
    pub truncated_groups: Vec<DID>,
    pub cycle_paths: Vec<Vec<DID>>,
    pub policy_digest: String,
    pub proof_digest: String,
}

pub const GROUP_EXPANSION_SNAPSHOT_OBJ_TYPE: &str = "buckyos.group_expansion_snapshot";
pub const GROUP_EXPANSION_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Profile / summary helpers used by the UI and ContactMgr projection.
// ---------------------------------------------------------------------------

/// Lightweight projection of a group, intended for member lists and home
/// screens. ContactMgr consumes this to expose hosted/joined groups in the
/// generic contact view without dragging the full `GroupDoc` everywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSummary {
    pub group_did: DID,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    pub host_zone: DID,
    pub owner: DID,
    pub purpose: GroupPurpose,
    pub entity_kind: DIDEntityKind,
    pub member_count: u32,
    pub is_hosted_by_self: bool,
    pub can_message: bool,
    pub updated_at_ms: u64,
}

/// Profile projection used by `get_group_profile()`. Carries enough info for
/// a UI "group home" screen without forcing it to read the full `GroupDoc`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupProfile {
    pub group_did: DID,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub host_zone: DID,
    pub owner: DID,
    pub entity_kind: DIDEntityKind,
    pub purpose: GroupPurpose,
    pub policy: GroupPolicy,
    pub collection_policy: GroupCollectionPolicy,
    pub public_membership: MembershipVisibility,
    pub public_admins: MembershipVisibility,
    pub member_count: u32,
    pub is_hosted_by_self: bool,
    pub can_message: bool,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupCreateProfile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(default)]
    pub purpose: GroupPurpose,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub group_did: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_zone: Option<DID>,
    #[serde(default)]
    pub public_membership: MembershipVisibility,
    #[serde(default)]
    pub public_admins: MembershipVisibility,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub endpoints: Option<GroupEndpoints>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupProfilePatch {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub purpose: Option<GroupPurpose>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub public_membership: Option<MembershipVisibility>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub public_admins: Option<MembershipVisibility>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub endpoints: Option<GroupEndpoints>,
}

// ---------------------------------------------------------------------------
// Group RPC wire types.
//
// Method strings live alongside the existing `METHOD_*` constants in
// `msg_center_client.rs`; the request structs are kept here to keep the
// shared model in one file. All requests carry a `contact_mgr_owner` style
// `host_owner` so a single msg-center can host groups for several zone users.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupCreateReq {
    pub owner_did: DID,
    pub profile: GroupCreateProfile,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub settings: Option<GroupSettings>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupGetDocReq {
    pub group_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupUpdateProfileReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub patch: GroupProfilePatch,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInviteMemberReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub member_did: DID,
    #[serde(default)]
    pub role: GroupRole,
    #[serde(default)]
    pub member_kind: DIDMemberKind,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub invite_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSubmitMemberProofReq {
    pub group_did: DID,
    pub proof: GroupMemberProof,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRequestJoinReq {
    pub member_did: DID,
    pub group_did: DID,
    pub proof: GroupMemberProof,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupApproveMemberReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub member_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRejectMemberReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub member_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRemoveMemberReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub member_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupUpdateMemberRoleReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub member_did: DID,
    pub role: GroupRole,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupListMembersReq {
    pub group_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub state_filter: Option<Vec<GroupMemberState>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role_filter: Option<Vec<GroupRole>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupCreateSubgroupReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub member_dids: Vec<DID>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSubgroupPatch {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub member_dids: Option<Vec<DID>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupUpdateSubgroupReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub subgroup_id: String,
    pub patch: GroupSubgroupPatch,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupListSubgroupsReq {
    pub group_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupUpdateCollectionPolicyReq {
    pub actor_did: DID,
    pub group_did: DID,
    pub policy: GroupCollectionPolicy,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupUpdateAttributionPolicyReq {
    pub actor_did: DID,
    pub group_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub policy: Option<GroupAttributionPolicy>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupExpandMembersReq {
    pub group_did: DID,
    pub purpose: GroupExpansionPurpose,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub actor_did: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_depth: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupListByMemberReq {
    pub member_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupListParentsReq {
    pub child_group_did: DID,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupCheckAccessReq {
    pub group_did: DID,
    pub actor_did: DID,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_owner: Option<DID>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupAccessDecision {
    pub action: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub effective_role: Option<GroupRole>,
}

/// Action codes used by `check_group_access` / RBAC. Kept as constants so
/// callers do not stringly-type them at every call-site.
pub mod group_action {
    pub const CREATE: &str = "group.create";
    pub const UPDATE_PROFILE: &str = "group.update_profile";
    pub const INVITE_MEMBER: &str = "group.invite_member";
    pub const APPROVE_MEMBER: &str = "group.approve_member";
    pub const REMOVE_MEMBER: &str = "group.remove_member";
    pub const UPDATE_ROLE: &str = "group.update_role";
    pub const MANAGE_SUBGROUP: &str = "group.manage_subgroup";
    pub const MANAGE_COLLECTION_POLICY: &str = "group.manage_collection_policy";
    pub const EXPAND_MEMBERS: &str = "group.expand_members";
    pub const UPDATE_ATTRIBUTION_POLICY: &str = "group.update_attribution_policy";
    pub const POST_MESSAGE: &str = "group.post_message";
    pub const READ_HISTORY: &str = "group.read_history";
    pub const ARCHIVE_OR_DELETE: &str = "group.archive_or_delete";
}

// ---------------------------------------------------------------------------
// JSON helpers.
// ---------------------------------------------------------------------------

pub fn parse_group_request<T: DeserializeOwned>(
    value: Value,
    type_name: &str,
) -> std::result::Result<T, RPCErrors> {
    serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParseRequestError(format!("failed to parse {}: {}", type_name, error))
    })
}
