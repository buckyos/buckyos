//! Relationship RPCs of the Contact Mgr (`msg_contact` in buckyos-api) and
//! who may call them: users act on their own store; a service may write only
//! the tags of its own namespace (`<service>.*`) and needs
//! `obj://msg-center/contact_tags/<service>` write for it.

use crate::msg_center::MessageCenter;
use buckyos_api::{
    contact_tag_app, contact_tag_kind, normalize_contact_tag, AccessGroupLevel,
    ContactCancelFriendRequestReq, ContactCheckMembershipReq, ContactCheckMembershipResult,
    ContactDidReq, ContactListFriendRequestsReq, ContactLookupReq, ContactRemoveFriendReq,
    ContactRespondFriendRequestReq, ContactSendFriendRequestReq, ContactSource, ContactTagKind,
    ContactUpdateTagsReq, TokenPrincipalKind, CONTACT_MEMBERSHIP_RBAC_RESOURCE,
    CONTACT_TAGS_RBAC_RESOURCE_PREFIX, METHOD_CONTACT_CANCEL_FRIEND_REQUEST,
    METHOD_CONTACT_CHECK_MEMBERSHIP, METHOD_CONTACT_LIST_FRIEND_REQUESTS,
    METHOD_CONTACT_LOOKUP, METHOD_CONTACT_REMOVE_CONTACT, METHOD_CONTACT_REMOVE_FRIEND,
    METHOD_CONTACT_RESPOND_FRIEND_REQUEST, METHOD_CONTACT_SEND_FRIEND_REQUEST,
    METHOD_CONTACT_UNBLOCK_CONTACT, METHOD_CONTACT_UPDATE_TAGS,
};
use kRPC::{RPCContext, RPCErrors};
use name_lib::DID;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

type Result<T> = std::result::Result<T, RPCErrors>;

fn parse<T: DeserializeOwned>(params: Value) -> Result<T> {
    serde_json::from_value(params).map_err(|error| RPCErrors::ParseRequestError(error.to_string()))
}

fn encode<T: Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| RPCErrors::ReasonError(error.to_string()))
}

fn denied(reason: &str) -> RPCErrors {
    RPCErrors::NoPermission(reason.to_string())
}

impl MessageCenter {
    /// The store an RPC acts on: the explicit owner, else the calling user.
    async fn contact_owner(&self, ctx: &RPCContext, explicit: Option<DID>) -> Result<DID> {
        if let Some(owner) = explicit {
            return Ok(owner);
        }
        self.caller_identity(ctx)
            .await?
            .and_then(|caller| caller.user_did)
            .ok_or_else(|| RPCErrors::ParseRequestError("contact_mgr_owner-required".into()))
    }

    /// The owner speaks through msg-center only when this Zone hosts it.
    async fn friend_owner(&self, ctx: &RPCContext, explicit: Option<DID>) -> Result<DID> {
        let owner = self.contact_owner(ctx, explicit).await?;
        self.authorize_contact_scope(ctx, Some(&owner), true).await?;
        if !self.is_local_recipient(&owner) {
            return Err(RPCErrors::ParseRequestError("owner-not-hosted".into()));
        }
        Ok(owner)
    }

    /// Who may change which tags; returns the source a missing contact gets.
    async fn authorize_tag_change(
        &self,
        ctx: &RPCContext,
        owner: &DID,
        tags: &[String],
    ) -> Result<ContactSource> {
        let Some(caller) = self.caller_identity(ctx).await? else {
            return Ok(ContactSource::AutoInferred);
        };
        if caller.principal_kind == TokenPrincipalKind::User {
            self.authorize_contact_scope(ctx, Some(owner), true).await?;
            if tags.iter().any(|tag| contact_tag_kind(tag) != ContactTagKind::User) {
                return Err(denied("tag-not-writable"));
            }
            return Ok(ContactSource::ManualCreate);
        }
        let service = caller
            .authorization_key
            .strip_prefix("system:")
            .ok_or_else(|| denied("tag-not-writable"))?
            .to_string();
        self.authorize_contact_scope(ctx, Some(owner), false).await?;
        if tags.iter().any(|tag| contact_tag_app(tag) != Some(service.as_str())) {
            return Err(denied("tag-not-writable"));
        }
        self.authorize_resource(
            ctx,
            &format!("{CONTACT_TAGS_RBAC_RESOURCE_PREFIX}{service}"),
            "write",
        )
        .await?;
        Ok(ContactSource::AutoInferred)
    }

    pub(crate) async fn contact_ext_rpc(
        &self,
        method: &str,
        params: Value,
        ctx: RPCContext,
    ) -> Result<Value> {
        match method {
            METHOD_CONTACT_LOOKUP => {
                let req: ContactLookupReq = parse(params)?;
                let owner = self.contact_owner(&ctx, req.contact_mgr_owner.clone()).await?;
                self.authorize_contact_scope(&ctx, Some(&owner), false)
                    .await?;
                encode(self.contact_lookup(&owner, req).await?)
            }
            METHOD_CONTACT_UPDATE_TAGS => {
                let req: ContactUpdateTagsReq = parse(params)?;
                let owner = self.contact_owner(&ctx, req.contact_mgr_owner.clone()).await?;
                let normalize = |tags: Vec<String>| -> Result<Vec<String>> {
                    tags.iter()
                        .map(|tag| {
                            normalize_contact_tag(tag)
                                .ok_or_else(|| RPCErrors::ParseRequestError("invalid-tag".into()))
                        })
                        .collect()
                };
                let add = normalize(req.add)?;
                let remove = normalize(req.remove)?;
                let all: Vec<String> = add.iter().chain(remove.iter()).cloned().collect();
                let source = self.authorize_tag_change(&ctx, &owner, &all).await?;
                encode(
                    self.contact_mgr
                        .update_tags(req.did, add, remove, source, Some(owner))
                        .await?,
                )
            }
            METHOD_CONTACT_REMOVE_CONTACT => {
                let req: ContactDidReq = parse(params)?;
                let owner = self.contact_owner(&ctx, req.contact_mgr_owner).await?;
                self.authorize_contact_scope(&ctx, Some(&owner), true).await?;
                let did = self
                    .contact_mgr
                    .resolve_canonical_did(req.did, Some(owner.clone()))
                    .await?;
                let contact = self
                    .contact_mgr
                    .get_contact(did.clone(), Some(owner.clone()))
                    .await?
                    .ok_or_else(|| denied("not-found"))?;
                if crate::contact_mgr::ContactMgr::is_system_relation(&contact) {
                    return Err(denied("system-relation"));
                }
                if self.is_local_recipient(&owner) {
                    self.settle_friendship(&owner, &did, true).await?;
                }
                let removed = self.contact_mgr.remove_contact(did, Some(owner)).await?;
                Ok(json!({"removed": true, "did": removed.did}))
            }
            METHOD_CONTACT_UNBLOCK_CONTACT => {
                let req: ContactDidReq = parse(params)?;
                let owner = self.contact_owner(&ctx, req.contact_mgr_owner).await?;
                self.authorize_contact_scope(&ctx, Some(&owner), true).await?;
                encode(self.contact_mgr.unblock_contact(req.did, Some(owner)).await?)
            }
            METHOD_CONTACT_SEND_FRIEND_REQUEST => {
                let req: ContactSendFriendRequestReq = parse(params)?;
                let owner = self.friend_owner(&ctx, req.contact_mgr_owner).await?;
                encode(self.send_friend_request(&owner, &req.did, req.message).await?)
            }
            METHOD_CONTACT_RESPOND_FRIEND_REQUEST => {
                let req: ContactRespondFriendRequestReq = parse(params)?;
                let owner = self.friend_owner(&ctx, req.contact_mgr_owner).await?;
                encode(
                    self.respond_friend_request(&owner, &req.request_id, req.decision)
                        .await?,
                )
            }
            METHOD_CONTACT_CANCEL_FRIEND_REQUEST => {
                let req: ContactCancelFriendRequestReq = parse(params)?;
                let owner = self.friend_owner(&ctx, req.contact_mgr_owner).await?;
                encode(self.cancel_friend_request(&owner, &req.request_id).await?)
            }
            METHOD_CONTACT_REMOVE_FRIEND => {
                let req: ContactRemoveFriendReq = parse(params)?;
                let owner = self.friend_owner(&ctx, req.contact_mgr_owner).await?;
                encode(
                    self.remove_friend(&owner, &req.did, req.notify.unwrap_or(true))
                        .await?,
                )
            }
            METHOD_CONTACT_LIST_FRIEND_REQUESTS => {
                let req: ContactListFriendRequestsReq = parse(params)?;
                let owner = self.contact_owner(&ctx, req.contact_mgr_owner.clone()).await?;
                self.authorize_contact_scope(&ctx, Some(&owner), false)
                    .await?;
                encode(self.list_friend_requests(&owner, &req).await?)
            }
            METHOD_CONTACT_CHECK_MEMBERSHIP => {
                let req: ContactCheckMembershipReq = parse(params)?;
                if let Some(caller) = self.caller_identity(&ctx).await? {
                    if caller.principal_kind == TokenPrincipalKind::User {
                        return Err(denied("service-only"));
                    }
                    self.authorize_resource(&ctx, CONTACT_MEMBERSHIP_RBAC_RESOURCE, "read")
                        .await?;
                }
                encode(self.check_contact_membership(&req).await?)
            }
            _ => Err(RPCErrors::UnknownMethod(method.to_string())),
        }
    }

    /// Is exactly `req.did` (no alias or merge resolution: the caller proved
    /// that DID) in `req.owner`'s group `req.tag`, projected Zone users
    /// included? A blocked contact is in no group.
    pub(crate) async fn check_contact_membership(
        &self,
        req: &ContactCheckMembershipReq,
    ) -> Result<ContactCheckMembershipResult> {
        let tag = normalize_contact_tag(&req.tag)
            .ok_or_else(|| RPCErrors::ParseRequestError("invalid-tag".into()))?;
        let contact = self
            .contact_mgr
            .get_contact(req.did.clone(), Some(req.owner.clone()))
            .await?
            .filter(|contact| self.contact_visible(Some(&req.owner), contact));
        Ok(ContactCheckMembershipResult {
            member: contact.is_some_and(|contact| {
                contact.access_level != AccessGroupLevel::Block
                    && contact.tags.iter().any(|t| *t == tag)
            }),
        })
    }
}
