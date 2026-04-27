//! Self-host group manager — see `doc/message_hub/Self-Host-Group.md`.
//!
//! Implementation choices:
//!
//! * Authoritative state is persisted in the shared msg-center sqlite/postgres
//!   database via `MsgBoxDbMgr`. Tables (`groups`, `group_members`,
//!   `group_member_proofs`, `group_subgroups`, `group_events`,
//!   `group_expansion_snapshots`) are declared in
//!   `buckyos_api::MSG_CENTER_RDB_SCHEMA_*`.
//! * The manager is multi-tenant: every row is scoped by `host_owner_key` so
//!   one msg-center process can host groups for several zone users (mirrors
//!   the `contacts.owner_key` model). `host_owner=None` is the system scope.
//! * In-memory caching is intentionally minimal — group state changes more
//!   slowly than the message stream, and the contact_mgr's snapshot model
//!   has shown how easy it is to corrupt with concurrent writes.
//! * Membership is two-way: `Active` requires a `GroupMemberProof`. Without
//!   a real signing layer the proof we accept is the canonical-JSON object,
//!   stored verbatim; verification is delegated to the proof signer.

use buckyos_api::{
    group_action, parse_group_request, DIDEntityKind, DIDMemberKind, ExpandedDID,
    GroupAccessDecision, GroupApproveMemberReq, GroupAttributionPolicy, GroupCheckAccessReq,
    GroupCollectionPolicy, GroupCreateReq, GroupCreateSubgroupReq, GroupDoc,
    GroupEndpoints, GroupEvent, GroupEventType, GroupExpandMembersReq, GroupExpansionPolicy,
    GroupExpansionPurpose, GroupExpansionSnapshot, GroupGetDocReq, GroupInviteMemberReq,
    GroupListByMemberReq, GroupListMembersReq, GroupListParentsReq, GroupListSubgroupsReq,
    GroupMemberProof, GroupMemberProofScope, GroupMemberRecord, GroupMemberState, GroupPolicy,
    GroupProfilePatch, GroupPurpose, GroupRejectMemberReq, GroupRemoveMemberReq, GroupRequestJoinReq,
    GroupRole, GroupSettings, GroupSubgroup, GroupSubgroupPatch, GroupSubmitMemberProofReq,
    GroupSummary, GroupUpdateAttributionPolicyReq, GroupUpdateCollectionPolicyReq,
    GroupUpdateMemberRoleReq, GroupUpdateProfileReq, GroupUpdateSubgroupReq, JoinPolicy,
    MembershipVisibility, NestedGroupPolicy, PostPolicy, RdbBackend, GROUP_DOC_OBJ_TYPE,
    GROUP_DOC_SCHEMA_VERSION, GROUP_EVENT_OBJ_TYPE, GROUP_EVENT_SCHEMA_VERSION,
    GROUP_EXPANSION_SNAPSHOT_OBJ_TYPE, GROUP_EXPANSION_SNAPSHOT_SCHEMA_VERSION,
};
use kRPC::RPCErrors;
use log::{debug, info};
use name_lib::DID;
use ndn_lib::ObjId;
use sqlx::{AnyPool, Row};
use std::collections::{HashMap, HashSet};

use crate::msg_box_db::MsgBoxDbMgr;

const SYSTEM_OWNER_SCOPE: &str = "__system__";
const DEFAULT_LIST_LIMIT: usize = 200;
const MAX_LIST_LIMIT: usize = 2000;
const MAX_EXPANSION_DEPTH: u8 = 8;

#[derive(Clone, Debug)]
pub struct GroupMgr {
    msg_box_db: MsgBoxDbMgr,
}

impl GroupMgr {
    pub fn new_with_msg_box(msg_box_db: MsgBoxDbMgr) -> Self {
        Self { msg_box_db }
    }

    fn pool(&self) -> &AnyPool {
        self.msg_box_db.pool()
    }

    fn render_sql(&self, sql: &str) -> String {
        match self.msg_box_db.backend() {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn owner_key(owner: Option<&DID>) -> String {
        owner
            .map(|did| did.to_string())
            .unwrap_or_else(|| SYSTEM_OWNER_SCOPE.to_string())
    }

    fn role_name(role: &GroupRole) -> &'static str {
        match role {
            GroupRole::Owner => "owner",
            GroupRole::Admin => "admin",
            GroupRole::Member => "member",
            GroupRole::Guest => "guest",
        }
    }

    fn member_state_name(state: &GroupMemberState) -> &'static str {
        match state {
            GroupMemberState::Invited => "invited",
            GroupMemberState::PendingMemberSignature => "pending_member_signature",
            GroupMemberState::PendingAdminApproval => "pending_admin_approval",
            GroupMemberState::Active => "active",
            GroupMemberState::Muted => "muted",
            GroupMemberState::Left => "left",
            GroupMemberState::Removed => "removed",
            GroupMemberState::Blocked => "blocked",
        }
    }

    fn member_kind_name(kind: &DIDMemberKind) -> &'static str {
        match kind {
            DIDMemberKind::Unknown => "unknown",
            DIDMemberKind::SingleEntity => "single_entity",
            DIDMemberKind::CollectionEntity => "collection_entity",
        }
    }

    fn parse_did(raw: &str, field: &str) -> std::result::Result<DID, RPCErrors> {
        DID::from_str(raw).map_err(|error| {
            RPCErrors::ReasonError(format!("failed to parse {} '{}': {}", field, raw, error))
        })
    }

    // ---------------------------------------------------------------------
    // 6.1 Create group.
    // ---------------------------------------------------------------------

    pub async fn create_group(
        &self,
        req: GroupCreateReq,
    ) -> std::result::Result<GroupDoc, RPCErrors> {
        let GroupCreateReq {
            owner_did,
            profile,
            settings,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        let now_ms = Self::now_ms();
        let host_zone = profile
            .host_zone
            .clone()
            .or_else(|| host_owner.clone())
            .unwrap_or_else(|| owner_did.clone());

        let group_did = match profile.group_did.as_ref() {
            Some(existing) => existing.clone(),
            None => Self::derive_group_did(&owner_did, &profile.name, &host_zone, now_ms),
        };

        // Refuse to clobber an existing group; callers must explicitly delete.
        if self.load_group_doc(&owner_key, &group_did).await?.is_some() {
            return Err(RPCErrors::ReasonError(format!(
                "group {} already exists for host_owner {}",
                group_did.to_string(),
                owner_key,
            )));
        }

        let endpoints = profile
            .endpoints
            .clone()
            .unwrap_or_else(|| Self::default_endpoints(&group_did));
        let mut effective_settings = settings.unwrap_or_else(|| GroupSettings::defaults_for(group_did.clone()));
        effective_settings.group_did = group_did.clone();

        let doc = GroupDoc {
            obj_type: GROUP_DOC_OBJ_TYPE.to_string(),
            schema_version: GROUP_DOC_SCHEMA_VERSION,
            group_did: group_did.clone(),
            entity_kind: DIDEntityKind::DIDCollection,
            did_doc_id: None,
            doc_version: 1,
            host_zone: host_zone.clone(),
            owner: owner_did.clone(),
            name: if profile.name.trim().is_empty() {
                group_did.to_string()
            } else {
                profile.name.trim().to_string()
            },
            avatar: profile.avatar.clone(),
            description: profile.description.clone(),
            purpose: profile.purpose.clone(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            profile_version: 1,
            policy: effective_settings.to_policy(),
            collection_policy: effective_settings.collection_policy.clone(),
            attribution_policy: effective_settings.attribution_policy.clone(),
            public_membership: profile.public_membership,
            public_admins: profile.public_admins,
            endpoints,
            proof: None,
        };

        self.persist_group(&owner_key, &doc, &effective_settings, true)
            .await?;

        // Owner is auto-active. The proof is host-issued (same Zone signed it),
        // mirroring the "same Zone auto-construction" path in §2.8.
        let owner_proof = self.issue_owner_self_proof(&group_did, &owner_did, now_ms);
        let owner_proof_id = self
            .persist_member_proof(&owner_key, &group_did, &owner_proof)
            .await?;

        let owner_record = GroupMemberRecord {
            group_did: group_did.clone(),
            member_did: owner_did.clone(),
            member_kind: DIDMemberKind::SingleEntity,
            role: GroupRole::Owner,
            state: GroupMemberState::Active,
            joined_at_ms: now_ms,
            updated_at_ms: now_ms,
            invited_by: None,
            approved_by: None,
            member_proof_id: Some(owner_proof_id),
            mute_until_ms: None,
            delivery_preference: None,
        };
        self.persist_member(&owner_key, &owner_record).await?;

        let event = self.build_event(
            &group_did,
            &owner_did,
            GroupEventType::GroupCreated,
            None,
            HashMap::new(),
        );
        self.persist_event(&owner_key, &event).await?;

        info!(
            "group created: host_owner={}, group_did={}, owner={}",
            owner_key,
            group_did.to_string(),
            owner_did.to_string(),
        );
        Ok(doc)
    }

    // ---------------------------------------------------------------------
    // 6.1.1 GroupDoc lookups / profile updates.
    // ---------------------------------------------------------------------

    pub async fn get_group_doc(
        &self,
        req: GroupGetDocReq,
    ) -> std::result::Result<Option<GroupDoc>, RPCErrors> {
        let owner_key = Self::owner_key(req.host_owner.as_ref());
        self.load_group_doc(&owner_key, &req.group_did).await
    }

    pub async fn update_group_profile(
        &self,
        req: GroupUpdateProfileReq,
    ) -> std::result::Result<GroupDoc, RPCErrors> {
        let GroupUpdateProfileReq {
            actor_did,
            group_did,
            patch,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(&owner_key, &group_did, &actor_did, group_action::UPDATE_PROFILE)
            .await?;

        let mut doc = self
            .load_group_doc(&owner_key, &group_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!("group not found: {}", group_did.to_string()))
            })?;
        let mut settings = self
            .load_group_settings(&owner_key, &group_did)
            .await?
            .unwrap_or_else(|| GroupSettings::defaults_for(group_did.clone()));

        let GroupProfilePatch {
            name,
            avatar,
            description,
            purpose,
            public_membership,
            public_admins,
            endpoints,
        } = patch;

        let now_ms = Self::now_ms();
        let mut profile_changed = false;
        if let Some(name) = name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                doc.name = trimmed.to_string();
                profile_changed = true;
            }
        }
        if let Some(avatar) = avatar {
            let trimmed = avatar.trim();
            doc.avatar = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
            profile_changed = true;
        }
        if let Some(description) = description {
            let trimmed = description.trim();
            doc.description = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
            profile_changed = true;
        }
        if let Some(purpose) = purpose {
            doc.purpose = purpose;
            profile_changed = true;
        }
        if let Some(visibility) = public_membership {
            doc.public_membership = visibility;
            profile_changed = true;
        }
        if let Some(visibility) = public_admins {
            doc.public_admins = visibility;
            profile_changed = true;
        }
        if let Some(endpoints) = endpoints {
            doc.endpoints = endpoints;
            profile_changed = true;
        }

        if profile_changed {
            doc.updated_at_ms = now_ms;
            doc.profile_version = doc.profile_version.saturating_add(1);
            doc.doc_version = doc.doc_version.saturating_add(1);
            self.persist_group(&owner_key, &doc, &settings, false)
                .await?;
            let event = self.build_event(
                &group_did,
                &actor_did,
                GroupEventType::ProfileUpdated,
                None,
                HashMap::new(),
            );
            self.persist_event(&owner_key, &event).await?;
        }

        // Settings remain unchanged here; saved to keep the projection stable.
        let _ = &mut settings;
        Ok(doc)
    }

    pub async fn update_collection_policy(
        &self,
        req: GroupUpdateCollectionPolicyReq,
    ) -> std::result::Result<GroupDoc, RPCErrors> {
        let GroupUpdateCollectionPolicyReq {
            actor_did,
            group_did,
            policy,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(
            &owner_key,
            &group_did,
            &actor_did,
            group_action::MANAGE_COLLECTION_POLICY,
        )
        .await?;
        let policy = sanitise_collection_policy(policy);

        let mut doc = self
            .load_group_doc(&owner_key, &group_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!("group not found: {}", group_did.to_string()))
            })?;
        let mut settings = self
            .load_group_settings(&owner_key, &group_did)
            .await?
            .unwrap_or_else(|| GroupSettings::defaults_for(group_did.clone()));

        doc.collection_policy = policy.clone();
        settings.collection_policy = policy;
        doc.doc_version = doc.doc_version.saturating_add(1);
        doc.updated_at_ms = Self::now_ms();
        self.persist_group(&owner_key, &doc, &settings, false)
            .await?;
        let event = self.build_event(
            &group_did,
            &actor_did,
            GroupEventType::CollectionPolicyUpdated,
            None,
            HashMap::new(),
        );
        self.persist_event(&owner_key, &event).await?;
        Ok(doc)
    }

    pub async fn update_attribution_policy(
        &self,
        req: GroupUpdateAttributionPolicyReq,
    ) -> std::result::Result<GroupDoc, RPCErrors> {
        let GroupUpdateAttributionPolicyReq {
            actor_did,
            group_did,
            policy,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(
            &owner_key,
            &group_did,
            &actor_did,
            group_action::UPDATE_ATTRIBUTION_POLICY,
        )
        .await?;
        let mut doc = self
            .load_group_doc(&owner_key, &group_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!("group not found: {}", group_did.to_string()))
            })?;
        let mut settings = self
            .load_group_settings(&owner_key, &group_did)
            .await?
            .unwrap_or_else(|| GroupSettings::defaults_for(group_did.clone()));

        doc.attribution_policy = policy.clone();
        settings.attribution_policy = policy;
        doc.doc_version = doc.doc_version.saturating_add(1);
        doc.updated_at_ms = Self::now_ms();
        self.persist_group(&owner_key, &doc, &settings, false)
            .await?;
        let event = self.build_event(
            &group_did,
            &actor_did,
            GroupEventType::AttributionPolicyUpdated,
            None,
            HashMap::new(),
        );
        self.persist_event(&owner_key, &event).await?;
        Ok(doc)
    }

    // ---------------------------------------------------------------------
    // 6.2 Invite member, 6.3 join request, member acceptance flow.
    // ---------------------------------------------------------------------

    pub async fn invite_member(
        &self,
        req: GroupInviteMemberReq,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        let GroupInviteMemberReq {
            actor_did,
            group_did,
            member_did,
            role,
            member_kind,
            invite_id,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(&owner_key, &group_did, &actor_did, group_action::INVITE_MEMBER)
            .await?;

        let doc = self
            .load_group_doc(&owner_key, &group_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!("group not found: {}", group_did.to_string()))
            })?;

        if matches!(role, GroupRole::Owner) {
            return Err(RPCErrors::ReasonError(
                "cannot invite as Owner; transfer ownership separately".to_string(),
            ));
        }

        // Refuse nested groups when policy disallows them.
        if matches!(member_kind, DIDMemberKind::CollectionEntity)
            && matches!(
                doc.collection_policy.nested_group_policy,
                NestedGroupPolicy::Disallow,
            )
        {
            return Err(RPCErrors::ReasonError(format!(
                "group {} disallows nested group members",
                group_did.to_string(),
            )));
        }

        // Already-active members short-circuit; everything else moves to Invited.
        if let Some(mut existing) = self.load_member(&owner_key, &group_did, &member_did).await? {
            if existing.state == GroupMemberState::Active {
                return Ok(existing);
            }
            existing.state = GroupMemberState::Invited;
            existing.role = role;
            existing.member_kind = member_kind;
            existing.invited_by = Some(actor_did.clone());
            existing.updated_at_ms = Self::now_ms();
            self.persist_member(&owner_key, &existing).await?;
            self.record_member_invited_event(&owner_key, &group_did, &actor_did, &member_did, invite_id.as_deref())
                .await?;
            return Ok(existing);
        }

        let now_ms = Self::now_ms();
        let record = GroupMemberRecord {
            group_did: group_did.clone(),
            member_did: member_did.clone(),
            member_kind,
            role,
            state: GroupMemberState::Invited,
            joined_at_ms: 0,
            updated_at_ms: now_ms,
            invited_by: Some(actor_did.clone()),
            approved_by: None,
            member_proof_id: None,
            mute_until_ms: None,
            delivery_preference: None,
        };
        self.persist_member(&owner_key, &record).await?;
        self.record_member_invited_event(&owner_key, &group_did, &actor_did, &member_did, invite_id.as_deref())
            .await?;
        Ok(record)
    }

    /// Submit a member-side proof of agreement to join. Drives an Invited
    /// member to Active (or PendingAdminApproval if the group requires it).
    pub async fn submit_member_proof(
        &self,
        req: GroupSubmitMemberProofReq,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        let GroupSubmitMemberProofReq {
            group_did,
            proof,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        let settings = self
            .load_group_settings(&owner_key, &group_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!("group not found: {}", group_did.to_string()))
            })?;
        Self::validate_member_proof(&group_did, &proof)?;

        let proof_id = self
            .persist_member_proof(&owner_key, &group_did, &proof)
            .await?;
        let now_ms = Self::now_ms();

        let mut record = match self
            .load_member(&owner_key, &group_did, &proof.member_did)
            .await?
        {
            Some(record) => record,
            None => {
                // Cold join request: write the member as PendingAdminApproval.
                let new_record = GroupMemberRecord {
                    group_did: group_did.clone(),
                    member_did: proof.member_did.clone(),
                    member_kind: proof.member_kind,
                    role: proof.role,
                    state: GroupMemberState::PendingAdminApproval,
                    joined_at_ms: 0,
                    updated_at_ms: now_ms,
                    invited_by: None,
                    approved_by: None,
                    member_proof_id: Some(proof_id.clone()),
                    mute_until_ms: None,
                    delivery_preference: None,
                };
                self.persist_member(&owner_key, &new_record).await?;
                let event = self.build_event(
                    &group_did,
                    &proof.member_did,
                    GroupEventType::MemberJoinRequested,
                    Some(proof.member_did.clone()),
                    HashMap::new(),
                );
                self.persist_event(&owner_key, &event).await?;
                return Ok(new_record);
            }
        };

        record.member_proof_id = Some(proof_id);
        record.member_kind = proof.member_kind;
        record.updated_at_ms = now_ms;
        let proof_event = self.build_event(
            &group_did,
            &proof.member_did,
            GroupEventType::MemberProofAccepted,
            Some(proof.member_did.clone()),
            HashMap::new(),
        );
        self.persist_event(&owner_key, &proof_event).await?;

        let needs_approval = matches!(settings.join_policy, JoinPolicy::RequestAndAdminApprove);
        if record.state == GroupMemberState::Invited && !needs_approval {
            record.state = GroupMemberState::Active;
            record.joined_at_ms = now_ms;
            self.persist_member(&owner_key, &record).await?;
            let joined_event = self.build_event(
                &group_did,
                &proof.member_did,
                GroupEventType::MemberJoined,
                Some(proof.member_did.clone()),
                HashMap::new(),
            );
            self.persist_event(&owner_key, &joined_event).await?;
            if matches!(record.member_kind, DIDMemberKind::CollectionEntity) {
                let nested_event = self.build_event(
                    &group_did,
                    &proof.member_did,
                    GroupEventType::NestedGroupAdded,
                    Some(proof.member_did.clone()),
                    HashMap::new(),
                );
                self.persist_event(&owner_key, &nested_event).await?;
            }
        } else if record.state == GroupMemberState::Invited && needs_approval {
            record.state = GroupMemberState::PendingAdminApproval;
            self.persist_member(&owner_key, &record).await?;
            let request_event = self.build_event(
                &group_did,
                &proof.member_did,
                GroupEventType::MemberApprovalRequested,
                Some(proof.member_did.clone()),
                HashMap::new(),
            );
            self.persist_event(&owner_key, &request_event).await?;
        } else {
            self.persist_member(&owner_key, &record).await?;
        }

        Ok(record)
    }

    /// Cold join request: a remote member (no prior invitation) submits
    /// their proof. The member transitions to PendingAdminApproval.
    pub async fn request_join(
        &self,
        req: GroupRequestJoinReq,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        let GroupRequestJoinReq {
            member_did,
            group_did,
            proof,
            host_owner,
        } = req;
        if member_did != proof.member_did {
            return Err(RPCErrors::ReasonError(
                "request_join member_did does not match proof.member_did".to_string(),
            ));
        }
        self.submit_member_proof(GroupSubmitMemberProofReq {
            group_did,
            proof,
            host_owner,
        })
        .await
    }

    pub async fn approve_member(
        &self,
        req: GroupApproveMemberReq,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        let GroupApproveMemberReq {
            actor_did,
            group_did,
            member_did,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(&owner_key, &group_did, &actor_did, group_action::APPROVE_MEMBER)
            .await?;

        let mut record = self
            .load_member(&owner_key, &group_did, &member_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "member {} not found in group {}",
                    member_did.to_string(),
                    group_did.to_string()
                ))
            })?;

        if record.member_proof_id.is_none() {
            return Err(RPCErrors::ReasonError(
                "cannot approve member without a member proof".to_string(),
            ));
        }

        record.state = GroupMemberState::Active;
        record.approved_by = Some(actor_did.clone());
        record.joined_at_ms = Self::now_ms();
        record.updated_at_ms = record.joined_at_ms;
        self.persist_member(&owner_key, &record).await?;

        let event = self.build_event(
            &group_did,
            &actor_did,
            GroupEventType::MemberJoined,
            Some(member_did.clone()),
            HashMap::new(),
        );
        self.persist_event(&owner_key, &event).await?;
        if matches!(record.member_kind, DIDMemberKind::CollectionEntity) {
            let nested_event = self.build_event(
                &group_did,
                &actor_did,
                GroupEventType::NestedGroupAdded,
                Some(member_did.clone()),
                HashMap::new(),
            );
            self.persist_event(&owner_key, &nested_event).await?;
        }
        Ok(record)
    }

    pub async fn reject_member(
        &self,
        req: GroupRejectMemberReq,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        let GroupRejectMemberReq {
            actor_did,
            group_did,
            member_did,
            reason,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(&owner_key, &group_did, &actor_did, group_action::APPROVE_MEMBER)
            .await?;

        let mut record = self
            .load_member(&owner_key, &group_did, &member_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "member {} not found in group {}",
                    member_did.to_string(),
                    group_did.to_string()
                ))
            })?;

        record.state = GroupMemberState::Removed;
        record.updated_at_ms = Self::now_ms();
        self.persist_member(&owner_key, &record).await?;

        let mut detail = HashMap::new();
        if let Some(reason) = reason.as_ref() {
            detail.insert("reason".to_string(), reason.clone());
        }
        let event = self.build_event(
            &group_did,
            &actor_did,
            GroupEventType::MemberRemoved,
            Some(member_did.clone()),
            detail,
        );
        self.persist_event(&owner_key, &event).await?;
        Ok(record)
    }

    pub async fn remove_member(
        &self,
        req: GroupRemoveMemberReq,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        let GroupRemoveMemberReq {
            actor_did,
            group_did,
            member_did,
            reason,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(&owner_key, &group_did, &actor_did, group_action::REMOVE_MEMBER)
            .await?;

        let mut record = self
            .load_member(&owner_key, &group_did, &member_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "member {} not found in group {}",
                    member_did.to_string(),
                    group_did.to_string()
                ))
            })?;

        if matches!(record.role, GroupRole::Owner) {
            return Err(RPCErrors::ReasonError(
                "cannot remove the owner; transfer ownership first".to_string(),
            ));
        }

        record.state = GroupMemberState::Removed;
        record.updated_at_ms = Self::now_ms();
        self.persist_member(&owner_key, &record).await?;

        let mut detail = HashMap::new();
        if let Some(reason) = reason.as_ref() {
            detail.insert("reason".to_string(), reason.clone());
        }
        let event = self.build_event(
            &group_did,
            &actor_did,
            GroupEventType::MemberRemoved,
            Some(member_did.clone()),
            detail,
        );
        self.persist_event(&owner_key, &event).await?;
        Ok(record)
    }

    pub async fn update_member_role(
        &self,
        req: GroupUpdateMemberRoleReq,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        let GroupUpdateMemberRoleReq {
            actor_did,
            group_did,
            member_did,
            role,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(&owner_key, &group_did, &actor_did, group_action::UPDATE_ROLE)
            .await?;

        let mut record = self
            .load_member(&owner_key, &group_did, &member_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "member {} not found in group {}",
                    member_did.to_string(),
                    group_did.to_string()
                ))
            })?;
        if matches!(record.role, GroupRole::Owner) || matches!(role, GroupRole::Owner) {
            return Err(RPCErrors::ReasonError(
                "owner role transfer must use a dedicated flow".to_string(),
            ));
        }
        record.role = role;
        record.updated_at_ms = Self::now_ms();
        self.persist_member(&owner_key, &record).await?;
        let event = self.build_event(
            &group_did,
            &actor_did,
            GroupEventType::RoleChanged,
            Some(member_did.clone()),
            HashMap::new(),
        );
        self.persist_event(&owner_key, &event).await?;
        Ok(record)
    }

    pub async fn list_members(
        &self,
        req: GroupListMembersReq,
    ) -> std::result::Result<Vec<GroupMemberRecord>, RPCErrors> {
        let owner_key = Self::owner_key(req.host_owner.as_ref());
        let mut records = self.load_members(&owner_key, &req.group_did).await?;

        if let Some(filter) = req.state_filter.as_ref() {
            let filter: HashSet<&'static str> = filter.iter().map(Self::member_state_name).collect();
            records.retain(|record| filter.contains(Self::member_state_name(&record.state)));
        }
        if let Some(filter) = req.role_filter.as_ref() {
            let filter: HashSet<&'static str> = filter.iter().map(Self::role_name).collect();
            records.retain(|record| filter.contains(Self::role_name(&record.role)));
        }
        records.sort_by(|left, right| left.member_did.to_string().cmp(&right.member_did.to_string()));

        let offset = req.offset.unwrap_or(0) as usize;
        let limit = req
            .limit
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .clamp(1, MAX_LIST_LIMIT);
        Ok(records.into_iter().skip(offset).take(limit).collect())
    }

    // ---------------------------------------------------------------------
    // 6.8 Subgroup management.
    // ---------------------------------------------------------------------

    pub async fn create_subgroup(
        &self,
        req: GroupCreateSubgroupReq,
    ) -> std::result::Result<GroupSubgroup, RPCErrors> {
        let GroupCreateSubgroupReq {
            actor_did,
            group_did,
            name,
            description,
            member_dids,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(&owner_key, &group_did, &actor_did, group_action::MANAGE_SUBGROUP)
            .await?;
        self.ensure_active_members(&owner_key, &group_did, &member_dids)
            .await?;
        let now_ms = Self::now_ms();
        let subgroup_id = Self::derive_subgroup_id(&name, now_ms);
        let subgroup = GroupSubgroup {
            group_did: group_did.clone(),
            subgroup_id: subgroup_id.clone(),
            name: name.trim().to_string(),
            description,
            member_dids: dedupe_dids(member_dids),
            created_by: actor_did.clone(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        self.persist_subgroup(&owner_key, &subgroup).await?;
        let mut detail = HashMap::new();
        detail.insert("subgroup_id".to_string(), subgroup_id);
        let event = self.build_event(
            &group_did,
            &actor_did,
            GroupEventType::SubgroupCreated,
            None,
            detail,
        );
        self.persist_event(&owner_key, &event).await?;
        Ok(subgroup)
    }

    pub async fn update_subgroup(
        &self,
        req: GroupUpdateSubgroupReq,
    ) -> std::result::Result<GroupSubgroup, RPCErrors> {
        let GroupUpdateSubgroupReq {
            actor_did,
            group_did,
            subgroup_id,
            patch,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());
        self.require_action(&owner_key, &group_did, &actor_did, group_action::MANAGE_SUBGROUP)
            .await?;

        let mut subgroup = self
            .load_subgroup(&owner_key, &group_did, &subgroup_id)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "subgroup {} not found in group {}",
                    subgroup_id,
                    group_did.to_string()
                ))
            })?;

        let GroupSubgroupPatch {
            name,
            description,
            member_dids,
        } = patch;

        let now_ms = Self::now_ms();
        if let Some(name) = name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                subgroup.name = trimmed.to_string();
            }
        }
        if let Some(description) = description {
            let trimmed = description.trim();
            subgroup.description = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(members) = member_dids {
            self.ensure_active_members(&owner_key, &group_did, &members)
                .await?;
            subgroup.member_dids = dedupe_dids(members);
        }
        subgroup.updated_at_ms = now_ms;
        self.persist_subgroup(&owner_key, &subgroup).await?;
        let mut detail = HashMap::new();
        detail.insert("subgroup_id".to_string(), subgroup.subgroup_id.clone());
        let event = self.build_event(
            &group_did,
            &actor_did,
            GroupEventType::SubgroupUpdated,
            None,
            detail,
        );
        self.persist_event(&owner_key, &event).await?;
        Ok(subgroup)
    }

    pub async fn list_subgroups(
        &self,
        req: GroupListSubgroupsReq,
    ) -> std::result::Result<Vec<GroupSubgroup>, RPCErrors> {
        let owner_key = Self::owner_key(req.host_owner.as_ref());
        self.load_subgroups(&owner_key, &req.group_did).await
    }

    // ---------------------------------------------------------------------
    // 6.10 Recursive expansion (bounded, with cycle detection).
    // ---------------------------------------------------------------------

    pub async fn expand_group_members(
        &self,
        req: GroupExpandMembersReq,
    ) -> std::result::Result<GroupExpansionSnapshot, RPCErrors> {
        let GroupExpandMembersReq {
            group_did,
            purpose,
            actor_did,
            max_depth,
            host_owner,
        } = req;
        let owner_key = Self::owner_key(host_owner.as_ref());

        let root_doc = self
            .load_group_doc(&owner_key, &group_did)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!("group not found: {}", group_did.to_string()))
            })?;
        let policy = root_doc.collection_policy.clone();
        let policy_digest = policy_digest_string(&policy);
        let max_depth = max_depth
            .unwrap_or(policy.max_expansion_depth.max(1))
            .min(MAX_EXPANSION_DEPTH);

        let allow_recursion = matches!(
            policy.nested_group_policy,
            NestedGroupPolicy::AllowWithRecursiveExpansion,
        ) && match purpose {
            GroupExpansionPurpose::Delivery => matches!(
                policy.expansion_policy,
                GroupExpansionPolicy::ExpandForDelivery
                    | GroupExpansionPolicy::ExpandForPermissionCheck
                    | GroupExpansionPolicy::ExpandForPublicView
            ),
            GroupExpansionPurpose::PermissionCheck => matches!(
                policy.expansion_policy,
                GroupExpansionPolicy::ExpandForPermissionCheck
                    | GroupExpansionPolicy::ExpandForPublicView
            ),
            GroupExpansionPurpose::PublicView => matches!(
                policy.expansion_policy,
                GroupExpansionPolicy::ExpandForPublicView
            ),
            GroupExpansionPurpose::Attribution => matches!(
                policy.expansion_policy,
                GroupExpansionPolicy::ExpandForPermissionCheck
                    | GroupExpansionPolicy::ExpandForPublicView
            ),
        };

        let mut expanded = Vec::new();
        let mut opaque = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut visited_groups: Vec<DID> = Vec::new();
        let mut truncated_groups: Vec<DID> = Vec::new();
        let mut cycle_paths: Vec<Vec<DID>> = Vec::new();

        self.expand_group_recursive(
            &owner_key,
            group_did.clone(),
            Vec::new(),
            0,
            max_depth,
            allow_recursion,
            policy.reject_cycles,
            &mut visited,
            &mut visited_groups,
            &mut truncated_groups,
            &mut cycle_paths,
            &mut expanded,
            &mut opaque,
        )
        .await?;

        let snapshot = GroupExpansionSnapshot {
            obj_type: GROUP_EXPANSION_SNAPSHOT_OBJ_TYPE.to_string(),
            schema_version: GROUP_EXPANSION_SNAPSHOT_SCHEMA_VERSION,
            operation_id: format!(
                "exp-{}-{}-{}",
                root_doc.group_did.to_raw_host_name(),
                Self::now_ms(),
                expanded.len() + opaque.len(),
            ),
            root_group_did: group_did.clone(),
            purpose,
            requested_by: actor_did,
            created_at_ms: Self::now_ms(),
            max_depth,
            expanded_members: expanded,
            opaque_members: opaque,
            visited_groups,
            truncated_groups,
            cycle_paths,
            policy_digest,
            proof_digest: format!("v1:{}", root_doc.doc_version),
        };

        self.persist_expansion_snapshot(&owner_key, &group_did, &snapshot)
            .await?;
        Ok(snapshot)
    }

    async fn expand_group_recursive(
        &self,
        owner_key: &str,
        group_did: DID,
        current_path: Vec<DID>,
        depth: u8,
        max_depth: u8,
        allow_recursion: bool,
        reject_cycles: bool,
        visited: &mut HashSet<String>,
        visited_groups: &mut Vec<DID>,
        truncated_groups: &mut Vec<DID>,
        cycle_paths: &mut Vec<Vec<DID>>,
        expanded: &mut Vec<ExpandedDID>,
        opaque: &mut Vec<DID>,
    ) -> std::result::Result<(), RPCErrors> {
        let key = group_did.to_string();
        if !visited.insert(key.clone()) {
            // Cycle hit on the current branch.
            if reject_cycles {
                let mut cycle = current_path.clone();
                cycle.push(group_did.clone());
                cycle_paths.push(cycle);
            }
            return Ok(());
        }
        visited_groups.push(group_did.clone());
        if depth >= max_depth {
            truncated_groups.push(group_did.clone());
            return Ok(());
        }

        let members = self.load_members(owner_key, &group_did).await?;
        for member in members {
            if !matches!(member.state, GroupMemberState::Active) {
                continue;
            }
            let mut path_for_child = current_path.clone();
            path_for_child.push(group_did.clone());

            match member.member_kind {
                DIDMemberKind::CollectionEntity if allow_recursion => {
                    Box::pin(self.expand_group_recursive(
                        owner_key,
                        member.member_did.clone(),
                        path_for_child.clone(),
                        depth + 1,
                        max_depth,
                        allow_recursion,
                        reject_cycles,
                        visited,
                        visited_groups,
                        truncated_groups,
                        cycle_paths,
                        expanded,
                        opaque,
                    ))
                    .await?;
                }
                DIDMemberKind::CollectionEntity => {
                    if !opaque.iter().any(|did| did == &member.member_did) {
                        opaque.push(member.member_did.clone());
                    }
                }
                _ => {
                    let exists = expanded.iter().any(|item| item.did == member.member_did);
                    if !exists {
                        expanded.push(ExpandedDID {
                            did: member.member_did.clone(),
                            member_kind: member.member_kind,
                            via_path: path_for_child,
                            role: Some(member.role),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Convenience used by MessageCenter dispatch: returns the flat list of
    /// active singleton-DID members, expanding nested groups when policy
    /// permits. Returns `None` when the group is unknown to GroupMgr (caller
    /// then falls back to `ContactMgr.get_group_subscribers`).
    pub async fn active_singleton_members(
        &self,
        owner_key: &str,
        group_did: &DID,
    ) -> std::result::Result<Option<Vec<DID>>, RPCErrors> {
        if self.load_group_doc(owner_key, group_did).await?.is_none() {
            return Ok(None);
        }
        let snapshot = self
            .expand_group_members(GroupExpandMembersReq {
                group_did: group_did.clone(),
                purpose: GroupExpansionPurpose::Delivery,
                actor_did: None,
                max_depth: None,
                host_owner: None, // owner_key is already chosen by caller
            })
            .await?;
        let mut dids: Vec<DID> = snapshot
            .expanded_members
            .into_iter()
            .map(|item| item.did)
            .collect();
        // Opaque nested groups are still valid recipients — host MessageCenter
        // will recursively dispatch into them.
        for did in snapshot.opaque_members {
            if !dids.iter().any(|existing| existing == &did) {
                dids.push(did);
            }
        }
        Ok(Some(dids))
    }

    // ---------------------------------------------------------------------
    // Lookup helpers used by ContactMgr / UI.
    // ---------------------------------------------------------------------

    pub async fn list_groups_by_member(
        &self,
        req: GroupListByMemberReq,
    ) -> std::result::Result<Vec<GroupSummary>, RPCErrors> {
        let owner_key = Self::owner_key(req.host_owner.as_ref());
        let sql = self.render_sql(
            "SELECT group_did FROM group_members WHERE host_owner_key = ? AND member_did = ?",
        );
        let rows = sqlx::query(&sql)
            .bind(owner_key.clone())
            .bind(req.member_did.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("query groups_by_member failed: {}", error))
            })?;

        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            let group_did_str: String = row
                .try_get("group_did")
                .map_err(|error| RPCErrors::ReasonError(format!("decode group_did failed: {}", error)))?;
            let group_did = Self::parse_did(&group_did_str, "group_did")?;
            if let Some(summary) = self.summarize_group(&owner_key, &group_did).await? {
                summaries.push(summary);
            }
        }
        Ok(summaries)
    }

    pub async fn list_parent_groups(
        &self,
        req: GroupListParentsReq,
    ) -> std::result::Result<Vec<GroupSummary>, RPCErrors> {
        // Same query as list_groups_by_member: the child group DID is in the
        // member_did column. The semantic split lives in GroupMemberRecord.
        // member_kind, which we filter on here.
        let owner_key = Self::owner_key(req.host_owner.as_ref());
        let sql = self.render_sql(
            "SELECT group_did FROM group_members WHERE host_owner_key = ? AND member_did = ? AND member_kind = ?",
        );
        let rows = sqlx::query(&sql)
            .bind(owner_key.clone())
            .bind(req.child_group_did.to_string())
            .bind(Self::member_kind_name(&DIDMemberKind::CollectionEntity).to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("query parent groups failed: {}", error))
            })?;
        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            let group_did_str: String = row
                .try_get("group_did")
                .map_err(|error| RPCErrors::ReasonError(format!("decode group_did failed: {}", error)))?;
            let group_did = Self::parse_did(&group_did_str, "group_did")?;
            if let Some(summary) = self.summarize_group(&owner_key, &group_did).await? {
                summaries.push(summary);
            }
        }
        Ok(summaries)
    }

    pub async fn is_group_did(
        &self,
        host_owner: Option<&DID>,
        did: &DID,
    ) -> std::result::Result<bool, RPCErrors> {
        let owner_key = Self::owner_key(host_owner);
        Ok(self.load_group_doc(&owner_key, did).await?.is_some())
    }

    pub async fn check_group_access(
        &self,
        req: GroupCheckAccessReq,
    ) -> std::result::Result<GroupAccessDecision, RPCErrors> {
        let owner_key = Self::owner_key(req.host_owner.as_ref());
        let outcome = self
            .resolve_action(&owner_key, &req.group_did, &req.actor_did, &req.action)
            .await?;
        Ok(GroupAccessDecision {
            action: req.action,
            allowed: outcome.allowed,
            reason: outcome.reason,
            effective_role: outcome.role,
        })
    }

    // ---------------------------------------------------------------------
    // RBAC / authorization helpers.
    // ---------------------------------------------------------------------

    async fn require_action(
        &self,
        owner_key: &str,
        group_did: &DID,
        actor_did: &DID,
        action: &str,
    ) -> std::result::Result<GroupRole, RPCErrors> {
        let outcome = self
            .resolve_action(owner_key, group_did, actor_did, action)
            .await?;
        if !outcome.allowed {
            return Err(RPCErrors::NoPermission(
                outcome
                    .reason
                    .unwrap_or_else(|| format!("actor not allowed: {}", action)),
            ));
        }
        outcome.role.ok_or_else(|| {
            RPCErrors::NoPermission(format!("actor {} has no group role", actor_did.to_string()))
        })
    }

    async fn resolve_action(
        &self,
        owner_key: &str,
        group_did: &DID,
        actor_did: &DID,
        action: &str,
    ) -> std::result::Result<ResolvedAction, RPCErrors> {
        let doc = self.load_group_doc(owner_key, group_did).await?;
        let Some(doc) = doc else {
            return Ok(ResolvedAction {
                allowed: false,
                reason: Some(format!("group not found: {}", group_did.to_string())),
                role: None,
            });
        };

        let member = self
            .load_member(owner_key, group_did, actor_did)
            .await?
            .filter(|record| matches!(record.state, GroupMemberState::Active));
        let role = member.as_ref().map(|record| record.role);

        let allowed = match action {
            group_action::POST_MESSAGE => Self::can_post(&doc.policy.post_policy, role.as_ref()),
            group_action::READ_HISTORY => member.is_some(),
            group_action::INVITE_MEMBER
            | group_action::APPROVE_MEMBER
            | group_action::REMOVE_MEMBER
            | group_action::UPDATE_PROFILE
            | group_action::UPDATE_ROLE
            | group_action::MANAGE_SUBGROUP
            | group_action::MANAGE_COLLECTION_POLICY
            | group_action::UPDATE_ATTRIBUTION_POLICY => matches!(
                role,
                Some(GroupRole::Owner) | Some(GroupRole::Admin)
            ),
            group_action::ARCHIVE_OR_DELETE => matches!(role, Some(GroupRole::Owner)),
            group_action::EXPAND_MEMBERS => member.is_some(),
            _ => matches!(role, Some(GroupRole::Owner) | Some(GroupRole::Admin)),
        };

        let reason = if allowed {
            None
        } else if role.is_none() {
            Some(format!(
                "actor {} is not an active member of {}",
                actor_did.to_string(),
                group_did.to_string()
            ))
        } else {
            Some(format!(
                "role {:?} is not permitted for {}",
                role, action
            ))
        };
        Ok(ResolvedAction { allowed, reason, role })
    }

    fn can_post(policy: &PostPolicy, role: Option<&GroupRole>) -> bool {
        let Some(role) = role else {
            return false;
        };
        match policy {
            PostPolicy::AllMembers => true,
            PostPolicy::AdminOnly => matches!(role, GroupRole::Owner | GroupRole::Admin),
            PostPolicy::RoleBased { roles } => roles.iter().any(|allowed| allowed == role),
        }
    }

    // ---------------------------------------------------------------------
    // Persistence — groups table.
    // ---------------------------------------------------------------------

    async fn persist_group(
        &self,
        owner_key: &str,
        doc: &GroupDoc,
        settings: &GroupSettings,
        is_create: bool,
    ) -> std::result::Result<(), RPCErrors> {
        let doc_json = serde_json::to_string(doc).map_err(|error| {
            RPCErrors::ReasonError(format!("encode group doc failed: {}", error))
        })?;
        let settings_json = serde_json::to_string(settings).map_err(|error| {
            RPCErrors::ReasonError(format!("encode group settings failed: {}", error))
        })?;

        let sql = if is_create {
            self.render_sql(
                "INSERT INTO groups (host_owner_key, group_did, doc_json, settings_json, is_hosted, updated_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
        } else {
            self.render_sql(
                "UPDATE groups SET doc_json = ?, settings_json = ?, is_hosted = ?, updated_at_ms = ?
                 WHERE host_owner_key = ? AND group_did = ?",
            )
        };

        let query = sqlx::query(&sql);
        let updated_at = doc.updated_at_ms as i64;
        let result = if is_create {
            query
                .bind(owner_key.to_string())
                .bind(doc.group_did.to_string())
                .bind(doc_json)
                .bind(settings_json)
                .bind(1_i64)
                .bind(updated_at)
                .execute(self.pool())
                .await
        } else {
            query
                .bind(doc_json)
                .bind(settings_json)
                .bind(1_i64)
                .bind(updated_at)
                .bind(owner_key.to_string())
                .bind(doc.group_did.to_string())
                .execute(self.pool())
                .await
        };
        result
            .map(|_| ())
            .map_err(|error| RPCErrors::ReasonError(format!("persist group failed: {}", error)))
    }

    async fn load_group_doc(
        &self,
        owner_key: &str,
        group_did: &DID,
    ) -> std::result::Result<Option<GroupDoc>, RPCErrors> {
        let sql = self.render_sql(
            "SELECT doc_json FROM groups WHERE host_owner_key = ? AND group_did = ?",
        );
        let row = sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(group_did.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("load group doc failed: {}", error))
            })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let raw: String = row
            .try_get("doc_json")
            .map_err(|error| RPCErrors::ReasonError(format!("decode doc_json failed: {}", error)))?;
        serde_json::from_str::<GroupDoc>(&raw).map(Some).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "parse group doc for {} failed: {}",
                group_did.to_string(),
                error
            ))
        })
    }

    async fn load_group_settings(
        &self,
        owner_key: &str,
        group_did: &DID,
    ) -> std::result::Result<Option<GroupSettings>, RPCErrors> {
        let sql = self.render_sql(
            "SELECT settings_json FROM groups WHERE host_owner_key = ? AND group_did = ?",
        );
        let row = sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(group_did.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("load group settings failed: {}", error))
            })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let raw: String = row.try_get("settings_json").map_err(|error| {
            RPCErrors::ReasonError(format!("decode settings_json failed: {}", error))
        })?;
        serde_json::from_str::<GroupSettings>(&raw).map(Some).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "parse group settings for {} failed: {}",
                group_did.to_string(),
                error
            ))
        })
    }

    async fn summarize_group(
        &self,
        owner_key: &str,
        group_did: &DID,
    ) -> std::result::Result<Option<GroupSummary>, RPCErrors> {
        let Some(doc) = self.load_group_doc(owner_key, group_did).await? else {
            return Ok(None);
        };
        let members = self.load_members(owner_key, group_did).await?;
        let active_count = members
            .iter()
            .filter(|record| matches!(record.state, GroupMemberState::Active))
            .count() as u32;
        Ok(Some(GroupSummary {
            group_did: doc.group_did.clone(),
            name: doc.name.clone(),
            avatar: doc.avatar.clone(),
            host_zone: doc.host_zone.clone(),
            owner: doc.owner.clone(),
            purpose: doc.purpose.clone(),
            entity_kind: doc.entity_kind,
            member_count: active_count,
            is_hosted_by_self: true,
            can_message: true,
            updated_at_ms: doc.updated_at_ms,
        }))
    }

    // ---------------------------------------------------------------------
    // Persistence — members.
    // ---------------------------------------------------------------------

    async fn persist_member(
        &self,
        owner_key: &str,
        record: &GroupMemberRecord,
    ) -> std::result::Result<(), RPCErrors> {
        let payload = serde_json::to_string(record).map_err(|error| {
            RPCErrors::ReasonError(format!("encode member record failed: {}", error))
        })?;
        let role = Self::role_name(&record.role).to_string();
        let state = Self::member_state_name(&record.state).to_string();
        let kind = Self::member_kind_name(&record.member_kind).to_string();
        let updated_at = record.updated_at_ms as i64;
        let sql = match self.msg_box_db.backend() {
            RdbBackend::Sqlite => self.render_sql(
                "INSERT INTO group_members
                 (host_owner_key, group_did, member_did, role, state, member_kind, record_json, updated_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(host_owner_key, group_did, member_did) DO UPDATE SET
                   role = excluded.role,
                   state = excluded.state,
                   member_kind = excluded.member_kind,
                   record_json = excluded.record_json,
                   updated_at_ms = excluded.updated_at_ms",
            ),
            RdbBackend::Postgres => self.render_sql(
                "INSERT INTO group_members
                 (host_owner_key, group_did, member_did, role, state, member_kind, record_json, updated_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (host_owner_key, group_did, member_did) DO UPDATE SET
                   role = EXCLUDED.role,
                   state = EXCLUDED.state,
                   member_kind = EXCLUDED.member_kind,
                   record_json = EXCLUDED.record_json,
                   updated_at_ms = EXCLUDED.updated_at_ms",
            ),
        };
        sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(record.group_did.to_string())
            .bind(record.member_did.to_string())
            .bind(role)
            .bind(state)
            .bind(kind)
            .bind(payload)
            .bind(updated_at)
            .execute(self.pool())
            .await
            .map(|_| ())
            .map_err(|error| RPCErrors::ReasonError(format!("persist member failed: {}", error)))
    }

    async fn load_member(
        &self,
        owner_key: &str,
        group_did: &DID,
        member_did: &DID,
    ) -> std::result::Result<Option<GroupMemberRecord>, RPCErrors> {
        let sql = self.render_sql(
            "SELECT record_json FROM group_members
             WHERE host_owner_key = ? AND group_did = ? AND member_did = ?",
        );
        let row = sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(group_did.to_string())
            .bind(member_did.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("load member failed: {}", error)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let raw: String = row
            .try_get("record_json")
            .map_err(|error| RPCErrors::ReasonError(format!("decode record_json failed: {}", error)))?;
        serde_json::from_str::<GroupMemberRecord>(&raw)
            .map(Some)
            .map_err(|error| {
                RPCErrors::ReasonError(format!("parse member record failed: {}", error))
            })
    }

    async fn load_members(
        &self,
        owner_key: &str,
        group_did: &DID,
    ) -> std::result::Result<Vec<GroupMemberRecord>, RPCErrors> {
        let sql = self.render_sql(
            "SELECT record_json FROM group_members
             WHERE host_owner_key = ? AND group_did = ?",
        );
        let rows = sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(group_did.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("load members failed: {}", error)))?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let raw: String = row.try_get("record_json").map_err(|error| {
                RPCErrors::ReasonError(format!("decode record_json failed: {}", error))
            })?;
            let record: GroupMemberRecord = serde_json::from_str(&raw).map_err(|error| {
                RPCErrors::ReasonError(format!("parse member record failed: {}", error))
            })?;
            records.push(record);
        }
        Ok(records)
    }

    async fn ensure_active_members(
        &self,
        owner_key: &str,
        group_did: &DID,
        member_dids: &[DID],
    ) -> std::result::Result<(), RPCErrors> {
        let active = self.load_members(owner_key, group_did).await?;
        let active_set: HashSet<String> = active
            .iter()
            .filter(|record| matches!(record.state, GroupMemberState::Active))
            .map(|record| record.member_did.to_string())
            .collect();
        for did in member_dids {
            if !active_set.contains(&did.to_string()) {
                return Err(RPCErrors::ReasonError(format!(
                    "subgroup member {} is not an active member of {}",
                    did.to_string(),
                    group_did.to_string(),
                )));
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Persistence — proofs.
    // ---------------------------------------------------------------------

    async fn persist_member_proof(
        &self,
        owner_key: &str,
        group_did: &DID,
        proof: &GroupMemberProof,
    ) -> std::result::Result<ObjId, RPCErrors> {
        let payload = serde_json::to_string(proof).map_err(|error| {
            RPCErrors::ReasonError(format!("encode member proof failed: {}", error))
        })?;
        let proof_id = Self::derive_proof_obj_id(proof);
        let sql = self.render_sql(
            "INSERT INTO group_member_proofs
             (host_owner_key, group_did, proof_id, member_did, payload_json, issued_at_ms)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(host_owner_key, group_did, proof_id) DO UPDATE SET
               member_did = excluded.member_did,
               payload_json = excluded.payload_json,
               issued_at_ms = excluded.issued_at_ms",
        );
        sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(group_did.to_string())
            .bind(proof_id.to_string())
            .bind(proof.member_did.to_string())
            .bind(payload)
            .bind(proof.issued_at_ms as i64)
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("persist member proof failed: {}", error))
            })?;
        Ok(proof_id)
    }

    fn issue_owner_self_proof(
        &self,
        group_did: &DID,
        owner_did: &DID,
        now_ms: u64,
    ) -> GroupMemberProof {
        GroupMemberProof {
            obj_type: buckyos_api::GROUP_MEMBER_PROOF_OBJ_TYPE.to_string(),
            schema_version: buckyos_api::GROUP_MEMBER_PROOF_SCHEMA_VERSION,
            group_did: group_did.clone(),
            member_did: owner_did.clone(),
            member_kind: DIDMemberKind::SingleEntity,
            role: GroupRole::Owner,
            proof_scope: GroupMemberProofScope::JoinAsSelf,
            invite_id: None,
            request_id: None,
            nonce: format!("owner-{}", now_ms),
            issued_at_ms: now_ms,
            expires_at_ms: None,
            signer: owner_did.clone(),
            member_zone: None,
            reverse_proof_uri: None,
            // Same-Zone auto-construction: the proof string records that the
            // host signed on behalf of the local owner, ready to be replaced
            // by a real DID Document signature once VerifyHub integration
            // lands. See §2.8.
            proof: format!("host-issued:owner:{}", owner_did.to_string()),
        }
    }

    fn validate_member_proof(
        group_did: &DID,
        proof: &GroupMemberProof,
    ) -> std::result::Result<(), RPCErrors> {
        if proof.group_did != *group_did {
            return Err(RPCErrors::ReasonError(format!(
                "proof.group_did {} does not match request {}",
                proof.group_did.to_string(),
                group_did.to_string(),
            )));
        }
        if proof.proof.trim().is_empty() {
            return Err(RPCErrors::ReasonError(
                "proof string is empty; refuse to mark member Active".to_string(),
            ));
        }
        if let Some(expires_at) = proof.expires_at_ms {
            if expires_at < Self::now_ms() {
                return Err(RPCErrors::ReasonError("proof has expired".to_string()));
            }
        }
        match proof.proof_scope {
            GroupMemberProofScope::JoinAsSelf => {
                if !matches!(proof.member_kind, DIDMemberKind::SingleEntity)
                    && !matches!(proof.member_kind, DIDMemberKind::Unknown)
                {
                    return Err(RPCErrors::ReasonError(
                        "JoinAsSelf proof must reference a single entity".to_string(),
                    ));
                }
            }
            GroupMemberProofScope::JoinAsCollectionEntity => {
                if !matches!(proof.member_kind, DIDMemberKind::CollectionEntity) {
                    return Err(RPCErrors::ReasonError(
                        "JoinAsCollectionEntity proof requires member_kind=CollectionEntity"
                            .to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Persistence — subgroups, events, expansion snapshots.
    // ---------------------------------------------------------------------

    async fn persist_subgroup(
        &self,
        owner_key: &str,
        subgroup: &GroupSubgroup,
    ) -> std::result::Result<(), RPCErrors> {
        let payload = serde_json::to_string(subgroup).map_err(|error| {
            RPCErrors::ReasonError(format!("encode subgroup failed: {}", error))
        })?;
        let sql = self.render_sql(
            "INSERT INTO group_subgroups
             (host_owner_key, group_did, subgroup_id, payload_json, updated_at_ms)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(host_owner_key, group_did, subgroup_id) DO UPDATE SET
               payload_json = excluded.payload_json,
               updated_at_ms = excluded.updated_at_ms",
        );
        sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(subgroup.group_did.to_string())
            .bind(subgroup.subgroup_id.clone())
            .bind(payload)
            .bind(subgroup.updated_at_ms as i64)
            .execute(self.pool())
            .await
            .map(|_| ())
            .map_err(|error| RPCErrors::ReasonError(format!("persist subgroup failed: {}", error)))
    }

    async fn load_subgroup(
        &self,
        owner_key: &str,
        group_did: &DID,
        subgroup_id: &str,
    ) -> std::result::Result<Option<GroupSubgroup>, RPCErrors> {
        let sql = self.render_sql(
            "SELECT payload_json FROM group_subgroups
             WHERE host_owner_key = ? AND group_did = ? AND subgroup_id = ?",
        );
        let row = sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(group_did.to_string())
            .bind(subgroup_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("load subgroup failed: {}", error)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let raw: String = row.try_get("payload_json").map_err(|error| {
            RPCErrors::ReasonError(format!("decode subgroup payload failed: {}", error))
        })?;
        serde_json::from_str::<GroupSubgroup>(&raw)
            .map(Some)
            .map_err(|error| {
                RPCErrors::ReasonError(format!("parse subgroup payload failed: {}", error))
            })
    }

    async fn load_subgroups(
        &self,
        owner_key: &str,
        group_did: &DID,
    ) -> std::result::Result<Vec<GroupSubgroup>, RPCErrors> {
        let sql = self.render_sql(
            "SELECT payload_json FROM group_subgroups
             WHERE host_owner_key = ? AND group_did = ? ORDER BY updated_at_ms DESC",
        );
        let rows = sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(group_did.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("load subgroups failed: {}", error)))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let raw: String = row.try_get("payload_json").map_err(|error| {
                RPCErrors::ReasonError(format!("decode subgroup payload failed: {}", error))
            })?;
            let subgroup: GroupSubgroup = serde_json::from_str(&raw).map_err(|error| {
                RPCErrors::ReasonError(format!("parse subgroup payload failed: {}", error))
            })?;
            out.push(subgroup);
        }
        Ok(out)
    }

    async fn persist_event(
        &self,
        owner_key: &str,
        event: &GroupEvent,
    ) -> std::result::Result<(), RPCErrors> {
        let payload = serde_json::to_string(event).map_err(|error| {
            RPCErrors::ReasonError(format!("encode group event failed: {}", error))
        })?;
        let event_type = format!("{:?}", event.event_type);
        let sql = self.render_sql(
            "INSERT INTO group_events
             (host_owner_key, group_did, event_id, event_type, payload_json, created_at_ms)
             VALUES (?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(event.group_did.to_string())
            .bind(event.event_id.clone())
            .bind(event_type)
            .bind(payload)
            .bind(event.created_at_ms as i64)
            .execute(self.pool())
            .await
            .map(|_| ())
            .map_err(|error| RPCErrors::ReasonError(format!("persist group event failed: {}", error)))
    }

    async fn record_member_invited_event(
        &self,
        owner_key: &str,
        group_did: &DID,
        actor_did: &DID,
        member_did: &DID,
        invite_id: Option<&str>,
    ) -> std::result::Result<(), RPCErrors> {
        let mut detail = HashMap::new();
        if let Some(invite_id) = invite_id {
            detail.insert("invite_id".to_string(), invite_id.to_string());
        }
        let event = self.build_event(
            group_did,
            actor_did,
            GroupEventType::MemberInvited,
            Some(member_did.clone()),
            detail,
        );
        self.persist_event(owner_key, &event).await
    }

    async fn persist_expansion_snapshot(
        &self,
        owner_key: &str,
        group_did: &DID,
        snapshot: &GroupExpansionSnapshot,
    ) -> std::result::Result<(), RPCErrors> {
        let payload = serde_json::to_string(snapshot).map_err(|error| {
            RPCErrors::ReasonError(format!("encode expansion snapshot failed: {}", error))
        })?;
        let sql = self.render_sql(
            "INSERT INTO group_expansion_snapshots
             (host_owner_key, group_did, operation_id, payload_json, created_at_ms)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(host_owner_key, group_did, operation_id) DO UPDATE SET
               payload_json = excluded.payload_json,
               created_at_ms = excluded.created_at_ms",
        );
        sqlx::query(&sql)
            .bind(owner_key.to_string())
            .bind(group_did.to_string())
            .bind(snapshot.operation_id.clone())
            .bind(payload)
            .bind(snapshot.created_at_ms as i64)
            .execute(self.pool())
            .await
            .map(|_| ())
            .map_err(|error| {
                RPCErrors::ReasonError(format!("persist expansion snapshot failed: {}", error))
            })
    }

    fn build_event(
        &self,
        group_did: &DID,
        actor_did: &DID,
        event_type: GroupEventType,
        target: Option<DID>,
        detail: HashMap<String, String>,
    ) -> GroupEvent {
        let now_ms = Self::now_ms();
        let event_id = format!(
            "{}-{}-{:?}",
            group_did.to_raw_host_name(),
            now_ms,
            event_type
        );
        let detail_btree: std::collections::BTreeMap<String, String> = detail.into_iter().collect();
        let event = GroupEvent {
            obj_type: GROUP_EVENT_OBJ_TYPE.to_string(),
            schema_version: GROUP_EVENT_SCHEMA_VERSION,
            event_id,
            group_did: group_did.clone(),
            actor: actor_did.clone(),
            event_type,
            target,
            created_at_ms: now_ms,
            detail: detail_btree,
        };
        debug!(
            "group event built: group_did={}, actor={}, type={:?}",
            event.group_did.to_string(),
            event.actor.to_string(),
            event.event_type,
        );
        event
    }

    // ---------------------------------------------------------------------
    // DID derivation helpers.
    // ---------------------------------------------------------------------

    fn derive_group_did(owner_did: &DID, name: &str, host_zone: &DID, now_ms: u64) -> DID {
        let owner_seed = owner_did.to_raw_host_name();
        let host_seed = host_zone.to_raw_host_name();
        let name_seed = sanitise_token(name);
        let subject = format!(
            "group-{}-{}-{}-{}",
            sanitise_token(&owner_seed),
            sanitise_token(&host_seed),
            name_seed,
            now_ms
        );
        DID::new("bns", &subject)
    }

    fn derive_subgroup_id(name: &str, now_ms: u64) -> String {
        format!("sg-{}-{}", sanitise_token(name), now_ms)
    }

    fn derive_proof_obj_id(proof: &GroupMemberProof) -> ObjId {
        // Stable object id derived from the canonical proof material.
        let key = format!(
            "proof:{}:{}:{}:{}",
            proof.group_did.to_string(),
            proof.member_did.to_string(),
            proof.nonce,
            proof.issued_at_ms,
        );
        let hash = simple_hash40(&key);
        // ObjId is parsed from a string of the form "obj:hash".
        let raw = format!("mobj:{}", hash);
        ObjId::new(&raw).unwrap_or_else(|_| {
            // Fallback should be impossible, but stay defensive.
            ObjId::new("mobj:0000000000000000000000000000000000000000").unwrap()
        })
    }

    fn default_endpoints(group_did: &DID) -> GroupEndpoints {
        let path = format!("/{}", group_did.to_raw_host_name());
        GroupEndpoints {
            message_center: None,
            group_mgr: None,
            inbox_path: Some(format!("{}/inbox", path)),
            subgroup_inbox_prefix: Some(format!("{}/sub/", path)),
            join_path: Some(format!("{}/join", path)),
            submit_member_proof_path: Some(format!("{}/member_proofs", path)),
            expand_members_path: Some(format!("{}/expand_members", path)),
            admin_operation_path: Some(format!("{}/admin", path)),
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedAction {
    allowed: bool,
    reason: Option<String>,
    role: Option<GroupRole>,
}

// ---------------------------------------------------------------------------
// Free helpers.
// ---------------------------------------------------------------------------

fn dedupe_dids(values: Vec<DID>) -> Vec<DID> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for did in values {
        if seen.insert(did.to_string()) {
            out.push(did);
        }
    }
    out
}

fn sanitise_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "g".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn simple_hash40(input: &str) -> String {
    // Cheap deterministic 40-char hex digest. We only need stability inside
    // this msg-center process; cryptographic strength is not required because
    // the proof object itself carries the real signature.
    let mut state: u64 = 0xcbf29ce484222325;
    let mut bytes: Vec<u8> = Vec::with_capacity(40);
    for ch in input.bytes() {
        state ^= ch as u64;
        state = state.wrapping_mul(0x100000001b3);
        bytes.push((state & 0xff) as u8);
        if bytes.len() >= 40 {
            break;
        }
    }
    while bytes.len() < 40 {
        state = state.wrapping_mul(0x100000001b3).wrapping_add(0x1234abcd);
        bytes.push((state & 0xff) as u8);
    }
    let mut out = String::with_capacity(40);
    for byte in bytes {
        out.push(hex_char((byte >> 4) & 0x0f));
        out.push(hex_char(byte & 0x0f));
    }
    out.chars().take(40).collect()
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => '0',
    }
}

fn rewrite_placeholders_to_dollar(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut idx = 0u32;
    let mut in_single = false;
    let mut in_double = false;
    for ch in sql.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(ch);
            }
            '?' if !in_single && !in_double => {
                idx += 1;
                out.push('$');
                out.push_str(&idx.to_string());
            }
            _ => out.push(ch),
        }
    }
    out
}

fn sanitise_collection_policy(mut policy: GroupCollectionPolicy) -> GroupCollectionPolicy {
    if policy.max_expansion_depth == 0 {
        policy.max_expansion_depth = 1;
    } else if policy.max_expansion_depth > MAX_EXPANSION_DEPTH {
        policy.max_expansion_depth = MAX_EXPANSION_DEPTH;
    }
    policy
}

fn policy_digest_string(policy: &GroupCollectionPolicy) -> String {
    let key = format!(
        "{:?}|{:?}|{}|{}",
        policy.nested_group_policy,
        policy.expansion_policy,
        policy.max_expansion_depth,
        policy.reject_cycles,
    );
    format!("policy:{}", simple_hash40(&key))
}

// ---------------------------------------------------------------------------
// Compatibility shim — accept JSON values directly so MessageCenter can hand
// off raw RPC params without re-encoding.
// ---------------------------------------------------------------------------

pub fn parse_request<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    type_name: &str,
) -> std::result::Result<T, RPCErrors> {
    parse_group_request(value, type_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        DIDMemberKind, GroupCreateProfile, GroupCreateReq, GroupExpandMembersReq,
        GroupExpansionPurpose, GroupInviteMemberReq, GroupListByMemberReq, GroupListMembersReq,
        GroupMemberProof, GroupMemberProofScope, GroupMemberState, GroupRole,
        GroupSubmitMemberProofReq, GROUP_MEMBER_PROOF_OBJ_TYPE,
        GROUP_MEMBER_PROOF_SCHEMA_VERSION,
    };
    use tempfile::tempdir;

    async fn new_test_mgr() -> (GroupMgr, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("groups.sqlite3");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
        let msg_box_db = MsgBoxDbMgr::open_default_sqlite(&conn).await.unwrap();
        let mgr = GroupMgr::new_with_msg_box(msg_box_db);
        (mgr, temp_dir)
    }

    fn member_proof(group_did: &DID, member_did: &DID, role: GroupRole) -> GroupMemberProof {
        GroupMemberProof {
            obj_type: GROUP_MEMBER_PROOF_OBJ_TYPE.to_string(),
            schema_version: GROUP_MEMBER_PROOF_SCHEMA_VERSION,
            group_did: group_did.clone(),
            member_did: member_did.clone(),
            member_kind: DIDMemberKind::SingleEntity,
            role,
            proof_scope: GroupMemberProofScope::JoinAsSelf,
            invite_id: None,
            request_id: None,
            nonce: format!("nonce-{}-{}", member_did.to_raw_host_name(), 1),
            issued_at_ms: GroupMgr::now_ms(),
            expires_at_ms: None,
            signer: member_did.clone(),
            member_zone: None,
            reverse_proof_uri: None,
            proof: format!("test-signed-by:{}", member_did.to_string()),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_group_and_owner_member_active() {
        let (mgr, _tmp) = new_test_mgr().await;
        let owner = DID::new("bns", "alice");
        let host_owner = owner.clone();

        let doc = mgr
            .create_group(GroupCreateReq {
                owner_did: owner.clone(),
                profile: GroupCreateProfile {
                    name: "Family".to_string(),
                    avatar: None,
                    description: Some("the family group".to_string()),
                    purpose: GroupPurpose::Conversation,
                    group_did: None,
                    host_zone: Some(owner.clone()),
                    public_membership: MembershipVisibility::MembersOnly,
                    public_admins: MembershipVisibility::MembersOnly,
                    endpoints: None,
                },
                settings: None,
                host_owner: Some(host_owner.clone()),
            })
            .await
            .unwrap();
        assert_eq!(doc.entity_kind, DIDEntityKind::DIDCollection);
        assert_eq!(doc.owner, owner);

        let members = mgr
            .list_members(GroupListMembersReq {
                group_did: doc.group_did.clone(),
                state_filter: None,
                role_filter: None,
                limit: None,
                offset: None,
                host_owner: Some(host_owner.clone()),
            })
            .await
            .unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].member_did, owner);
        assert_eq!(members[0].state, GroupMemberState::Active);
        assert_eq!(members[0].role, GroupRole::Owner);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invite_then_proof_marks_member_active_when_open_join() {
        let (mgr, _tmp) = new_test_mgr().await;
        let owner = DID::new("bns", "alice");
        let host_owner = owner.clone();
        let bob = DID::new("bns", "bob");

        let doc = mgr
            .create_group(GroupCreateReq {
                owner_did: owner.clone(),
                profile: GroupCreateProfile {
                    name: "Project".to_string(),
                    avatar: None,
                    description: None,
                    purpose: GroupPurpose::Collaboration,
                    group_did: None,
                    host_zone: Some(owner.clone()),
                    public_membership: MembershipVisibility::MembersOnly,
                    public_admins: MembershipVisibility::MembersOnly,
                    endpoints: None,
                },
                settings: None,
                host_owner: Some(host_owner.clone()),
            })
            .await
            .unwrap();

        let invited = mgr
            .invite_member(GroupInviteMemberReq {
                actor_did: owner.clone(),
                group_did: doc.group_did.clone(),
                member_did: bob.clone(),
                role: GroupRole::Member,
                member_kind: DIDMemberKind::SingleEntity,
                invite_id: Some("inv-1".to_string()),
                host_owner: Some(host_owner.clone()),
            })
            .await
            .unwrap();
        assert_eq!(invited.state, GroupMemberState::Invited);

        let proof = member_proof(&doc.group_did, &bob, GroupRole::Member);
        let activated = mgr
            .submit_member_proof(GroupSubmitMemberProofReq {
                group_did: doc.group_did.clone(),
                proof,
                host_owner: Some(host_owner.clone()),
            })
            .await
            .unwrap();
        assert_eq!(activated.state, GroupMemberState::Active);
        assert!(activated.member_proof_id.is_some());

        let by_member = mgr
            .list_groups_by_member(GroupListByMemberReq {
                member_did: bob.clone(),
                host_owner: Some(host_owner.clone()),
            })
            .await
            .unwrap();
        assert_eq!(by_member.len(), 1);
        assert_eq!(by_member[0].group_did, doc.group_did);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn expand_members_returns_singletons_only_by_default() {
        let (mgr, _tmp) = new_test_mgr().await;
        let owner = DID::new("bns", "alice");
        let host_owner = owner.clone();
        let bob = DID::new("bns", "bob");

        let doc = mgr
            .create_group(GroupCreateReq {
                owner_did: owner.clone(),
                profile: GroupCreateProfile {
                    name: "Family".to_string(),
                    avatar: None,
                    description: None,
                    purpose: GroupPurpose::Conversation,
                    group_did: None,
                    host_zone: Some(owner.clone()),
                    public_membership: MembershipVisibility::MembersOnly,
                    public_admins: MembershipVisibility::MembersOnly,
                    endpoints: None,
                },
                settings: None,
                host_owner: Some(host_owner.clone()),
            })
            .await
            .unwrap();
        mgr.invite_member(GroupInviteMemberReq {
            actor_did: owner.clone(),
            group_did: doc.group_did.clone(),
            member_did: bob.clone(),
            role: GroupRole::Member,
            member_kind: DIDMemberKind::SingleEntity,
            invite_id: None,
            host_owner: Some(host_owner.clone()),
        })
        .await
        .unwrap();
        mgr.submit_member_proof(GroupSubmitMemberProofReq {
            group_did: doc.group_did.clone(),
            proof: member_proof(&doc.group_did, &bob, GroupRole::Member),
            host_owner: Some(host_owner.clone()),
        })
        .await
        .unwrap();

        let snapshot = mgr
            .expand_group_members(GroupExpandMembersReq {
                group_did: doc.group_did.clone(),
                purpose: GroupExpansionPurpose::Delivery,
                actor_did: None,
                max_depth: None,
                host_owner: Some(host_owner.clone()),
            })
            .await
            .unwrap();
        let dids: HashSet<String> = snapshot
            .expanded_members
            .iter()
            .map(|item| item.did.to_string())
            .collect();
        assert!(dids.contains(&owner.to_string()));
        assert!(dids.contains(&bob.to_string()));
        assert!(snapshot.cycle_paths.is_empty());
    }
}
