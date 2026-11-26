use crate::group_store::db_error;
use crate::group_sync::JoinedGroupRoute;
use crate::group_types::*;
use crate::msg_center::MessageCenter;
use crate::network_directory::DirectoryError;
use buckyos_api::LookupKind;
use kRPC::RPCContext;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct JoinedGroupBinding {
    schema_version: u32,
    owner_did: DID,
    group_did: DID,
    pub(crate) host: String,
    pending_invitation_id: Option<String>,
    /// A join request (`group.request_join`) the host has not answered yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_request: Option<PendingJoinRequest>,
    member: Option<Value>,
    doc_cache: Option<Value>,
}

/// The host answers the same `idempotency_key` with the outcome it saved, so asking again
/// after a lost reply or a restart neither repeats the request nor uses an invite link twice.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingJoinRequest {
    idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invite: Option<String>,
}

/// An invitation into a group hosted elsewhere, as its notice told this Zone (the inviter is
/// the notice's proven sender; the rest is checked by the host on acceptance).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReceivedInvitation {
    schema_version: u32,
    group_did: DID,
    invitation_id: String,
    invited_by: DID,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    member_did: Option<DID>,
    role: GroupRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at_ms: Option<u64>,
    #[serde(default)]
    profile: Value,
    received_at_ms: u64,
}

const MAX_RECEIVED_INVITATIONS: usize = 256;
const MAX_INVITATION_PROFILE_BYTES: usize = 16 * 1024;

/// Host answers that end an accept or a request for good: the binding forgets it.
fn is_final_join_answer(error: &kRPC::RPCErrors) -> bool {
    matches!(error, kRPC::RPCErrors::NoPermission(reason) if matches!(reason.as_str(),
        "invitation-mismatch" | "invitation-expired" | "invitation-revoked" | "not-found" | "remote-group-not-found" | "blocked" | "client-not-allowed"))
}

impl JoinedGroupBinding {
    fn new(owner_did: DID, group_did: DID, host: String) -> Self {
        Self {
            schema_version: 1,
            owner_did,
            group_did,
            host,
            pending_invitation_id: None,
            pending_request: None,
            member: None,
            doc_cache: None,
        }
    }

    /// An accept or a join request was sent and the host has not answered it yet.
    fn unanswered(&self) -> bool {
        self.pending_invitation_id.is_some() || self.pending_request.is_some()
    }

    fn pending_approval(&self) -> bool {
        self.member
            .as_ref()
            .is_some_and(|member| member["state"] == "pending_admin_approval")
    }

    pub(crate) fn active(&self) -> bool {
        self.member
            .as_ref()
            .is_some_and(|member| member["state"] == "active")
    }

    pub(crate) fn epoch(&self) -> Option<u32> {
        self.member
            .as_ref()?
            .get("epoch")?
            .as_u64()?
            .try_into()
            .ok()
    }

    pub(crate) fn route(&self, center: &MessageCenter) -> JoinedGroupRoute {
        JoinedGroupRoute {
            owner_did: self.owner_did.clone(),
            group_did: self.group_did.clone(),
            host: self.host.clone(),
            upstream: center
                .cyfs_dispatch
                .read()
                .unwrap()
                .origin_of_zone(&self.host),
            authorization: None,
        }
    }
}

impl MessageCenter {
    pub(crate) async fn native_message_route(
        &self,
        msg: &ndn_lib::MsgObject,
        receiver: &DID,
    ) -> anyhow::Result<Option<crate::cyfs_dispatch::NativeDispatchRoute>> {
        if msg.kind == ndn_lib::MsgObjKind::GroupMsg {
            let route = self
                .joined_group_route(&msg.from, receiver)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?
                .ok_or_else(|| anyhow::anyhow!("joined-group-route-not-configured"))?;
            return Ok(Some(route.message_route(msg)?));
        }
        Ok(self
            .cyfs_dispatch
            .read()
            .unwrap()
            .outgoing
            .get(&receiver.to_string())
            .cloned())
    }
    pub(crate) async fn resolve_group_route(
        &self,
        owner: &DID,
        group: &DID,
        refresh_negative: bool,
    ) -> Result<JoinedGroupRoute> {
        let profile = self
            .directory
            .get()
            .describe(group, refresh_negative)
            .await
            .map_err(|error| match error {
                DirectoryError::NotFound(_) => denied("remote-group-not-found"),
                _ => kRPC::RPCErrors::ReasonError("remote-group-unavailable".into()),
            })?;
        if profile.kind != LookupKind::Group {
            return Err(missing());
        }
        let host = profile.zone.ok_or_else(missing)?;
        if self
            .cyfs_dispatch
            .read()
            .unwrap()
            .target_zone
            .as_deref()
            .is_some_and(|local| local.eq_ignore_ascii_case(&host))
        {
            return Err(missing());
        }
        let route = JoinedGroupRoute {
            owner_did: owner.clone(),
            group_did: group.clone(),
            host: host.clone(),
            upstream: self.cyfs_dispatch.read().unwrap().origin_of_zone(&host),
            authorization: None,
        };
        route.validate()?;
        Ok(route)
    }

    pub(crate) async fn load_group_binding(
        &self,
        owner: &DID,
        group: &DID,
    ) -> Result<Option<JoinedGroupBinding>> {
        let sql = self.msg_box_db.render_sql(
            "SELECT binding_json FROM group_joined_bindings WHERE owner=? AND group_did=?",
        );
        let row = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(group.to_string())
            .fetch_optional(self.msg_box_db.pool())
            .await
            .map_err(db_error)?;
        row.map(|row| {
            let raw: String = row.try_get("binding_json").map_err(db_error)?;
            serde_json::from_str(&raw).map_err(db_error)
        })
        .transpose()
    }

    async fn save_group_binding(&self, binding: &JoinedGroupBinding, reset: bool) -> Result<()> {
        let _guard = self
            .groups
            .sync_locks
            .lock(&binding.owner_did, &binding.group_did)
            .await;
        let mut tx = self.msg_box_db.pool().begin().await.map_err(db_error)?;
        let sql = self.msg_box_db.render_sql("INSERT INTO group_joined_bindings(owner,group_did,binding_json) VALUES(?,?,?) ON CONFLICT(owner,group_did) DO UPDATE SET binding_json=excluded.binding_json");
        sqlx::query(&sql)
            .bind(binding.owner_did.to_string())
            .bind(binding.group_did.to_string())
            .bind(serde_json::to_string(binding).map_err(db_error)?)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        if reset {
            let sql = self
                .msg_box_db
                .render_sql("SELECT state_json FROM joined_groups WHERE owner=? AND group_did=?");
            if let Some(row) = sqlx::query(&sql)
                .bind(binding.owner_did.to_string())
                .bind(binding.group_did.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_error)?
            {
                let raw: String = row.try_get("state_json").map_err(db_error)?;
                let mut state: crate::group_sync::JoinedGroupState =
                    serde_json::from_str(&raw).map_err(db_error)?;
                state.stopped = false;
                state.changes_token = None;
                state.recent_push_tokens.clear();
                state.member_epoch = binding.epoch();
                state.session_cache.clear();
                self.store_joined_state(&mut tx, &state, Some(&raw)).await?;
            }
        }
        tx.commit().await.map_err(db_error)?;
        Ok(())
    }

    async fn group_bindings(&self, owner: Option<&DID>) -> Result<Vec<JoinedGroupBinding>> {
        let rows = if let Some(owner) = owner {
            let sql = self.msg_box_db.render_sql(
                "SELECT binding_json FROM group_joined_bindings WHERE owner=? ORDER BY group_did",
            );
            sqlx::query(&sql)
                .bind(owner.to_string())
                .fetch_all(self.msg_box_db.pool())
                .await
        } else {
            sqlx::query("SELECT binding_json FROM group_joined_bindings ORDER BY owner,group_did")
                .fetch_all(self.msg_box_db.pool())
                .await
        }
        .map_err(db_error)?;
        rows.into_iter()
            .map(|row| {
                let raw: String = row.try_get("binding_json").map_err(db_error)?;
                serde_json::from_str(&raw).map_err(db_error)
            })
            .collect()
    }

    pub(crate) async fn joined_group_route(
        &self,
        owner: &DID,
        group: &DID,
    ) -> Result<Option<JoinedGroupRoute>> {
        if let Some(binding) = self.load_group_binding(owner, group).await? {
            if binding.active() {
                if self
                    .joined_state(owner, group)
                    .await?
                    .is_some_and(|state| state.stopped)
                {
                    return Err(denied("group-membership-ended"));
                }
                return Ok(Some(binding.route(self)));
            }
            return Err(denied("member-not-active"));
        }
        Ok(self.configured_group_route(owner, group))
    }

    pub(crate) fn configured_group_route(
        &self,
        owner: &DID,
        group: &DID,
    ) -> Option<JoinedGroupRoute> {
        self.cyfs_dispatch
            .read()
            .unwrap()
            .joined_groups
            .iter()
            .find(|route| route.owner_did == *owner && route.group_did == *group)
            .cloned()
    }

    /// The group itself telling `owner` about their place in it (`msg.from` is proven by the
    /// transport), and this Zone knows `owner` joined it or asked to: it goes to the group's
    /// session, past the contact policy. A group nobody here asked for stays a stranger.
    pub(crate) async fn is_notice_of_own_group(&self, msg: &ndn_lib::MsgObject, owner: &DID) -> bool {
        let names_itself = msg.kind == ndn_lib::MsgObjKind::Operation
            && msg.content.machine.as_ref().is_some_and(|machine| {
                machine.intent.as_deref() == Some(GROUP_NOTICE_INTENT)
                    && serde_json::to_value(&machine.data).is_ok_and(|data| data["group_did"] == json!(msg.from))
            });
        if !names_itself {
            return false;
        }
        if matches!(self.load_group_binding(owner, &msg.from).await, Ok(Some(_))) {
            return true;
        }
        self.cyfs_dispatch
            .read()
            .unwrap()
            .joined_groups
            .iter()
            .any(|route| route.owner_did == *owner && route.group_did == msg.from)
    }

    pub(crate) async fn remote_group_doc(&self, owner: &DID, group: &DID) -> Result<Value> {
        let route = self.resolve_group_route(owner, group, false).await?;
        let value = route
            .fetch(self, &format!("{}/doc", route.base_path()), &[])
            .await?;
        if value["doc"]["id"] != json!(group) {
            return Err(invalid("invalid-group-response"));
        }
        Ok(value)
    }

    /// The member list (`session: None`) or one Session's participants of a joined group, as
    /// its host shows them to `owner`.
    pub(crate) async fn remote_group_members(
        &self,
        owner: &DID,
        group: &DID,
        session: Option<Option<&str>>,
    ) -> Result<Value> {
        let route = self
            .joined_group_route(owner, group)
            .await?
            .ok_or_else(missing)?;
        let path = match session {
            None => format!("{}/members", route.base_path()),
            Some(None) => format!("{}/participants", route.base_path()),
            Some(Some(sid)) => {
                let key = session_key(group, Some(sid))?;
                let (_, encoded) = key
                    .split_once('/')
                    .ok_or_else(|| invalid("invalid-session-id"))?;
                format!("{}/sessions/{}/participants", route.base_path(), encoded)
            }
        };
        route.fetch(self, &path, &[]).await
    }

    /// Joining a group hosted elsewhere is for this Zone's own signed-in people (and their
    /// Agents): no External or restricted session, no remote caller, no tunnel attestation.
    async fn check_remote_join_caller(
        &self,
        actor: &GroupActor,
        p: &Value,
        ctx: &RPCContext,
    ) -> Result<()> {
        let caller = self
            .caller_identity(ctx)
            .await?
            .ok_or_else(|| denied("authentication-required"))?;
        if caller.external
            || caller.restricted()
            || actor.remote
            || !self.is_hosted_identity(&actor.did)
            || p.get("attestation").is_some()
        {
            return Err(denied("remote-group-auth-failed"));
        }
        Ok(())
    }

    pub(crate) async fn accept_remote_group_invitation(
        &self,
        group: &DID,
        actor: &GroupActor,
        p: &Value,
        ctx: &RPCContext,
    ) -> Result<Value> {
        self.check_remote_join_caller(actor, p, ctx).await?;
        let invitation = p["invitation_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| invalid("missing-invitation_id"))?;
        let member: DID = match p.get("member_did").filter(|v| !v.is_null()) {
            Some(value) => serde_json::from_value(value.clone()).map_err(invalid)?,
            None => actor.did.clone(),
        };
        if member != actor.did
            && self
                .token_verifier
                .get()
                .agent_owner(&member)
                .await?
                .as_ref()
                != Some(&actor.did)
        {
            return Err(denied("agent-owner-required"));
        }
        self.require_group_enabled_agent(&member).await?;
        if !self.is_hosted_identity(&member) {
            return Err(denied("remote-group-auth-failed"));
        }
        let _guard = self.groups.join_locks.lock(&member, group).await;
        let route = self.resolve_group_route(&member, group, true).await?;
        let mut saved = self.load_group_binding(&member, group).await?;
        if saved.as_ref().is_some_and(|binding| {
            binding.pending_request.is_some()
                || binding
                    .pending_invitation_id
                    .as_deref()
                    .is_some_and(|pending| pending != invitation)
        }) {
            let outcome = self.resume_group_binding(saved.take().unwrap()).await;
            saved = self.load_group_binding(&member, group).await?;
            if let Err(error) = outcome {
                if saved.as_ref().is_some_and(JoinedGroupBinding::unanswered) {
                    return Err(error);
                }
            }
        }
        let mut binding = saved.unwrap_or_else(|| {
            JoinedGroupBinding::new(member.clone(), group.clone(), route.host.clone())
        });
        let host_changed = !binding.host.eq_ignore_ascii_case(&route.host);
        if host_changed {
            binding.member = None;
            binding.doc_cache = None;
        }
        binding.host = route.host;
        binding.pending_invitation_id = Some(invitation.into());
        self.save_group_binding(&binding, host_changed).await?;
        let result = self.resume_group_binding(binding).await;
        match &result {
            Ok(_) => self.forget_received_invitation(&member, group, None).await,
            Err(error) if is_final_join_answer(error) => {
                self.forget_received_invitation(&member, group, Some(invitation))
                    .await
            }
            Err(_) => {}
        }
        result
    }

    /// `group.request_join` for a group hosted elsewhere: the host decides by its join policy
    /// or the invite link. Persisted before it is sent, and followed up like an accepted
    /// invitation: the host gives the request an id in the member record's `invitation_id`.
    pub(crate) async fn request_remote_group_join(
        &self,
        group: &DID,
        actor: &GroupActor,
        p: &Value,
        ctx: &RPCContext,
    ) -> Result<Value> {
        self.check_remote_join_caller(actor, p, ctx).await?;
        let invite = match p.get("invite").filter(|v| !v.is_null()) {
            Some(value) => Some(
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|token| !token.is_empty() && token.len() <= 512)
                    .ok_or_else(|| invalid("invalid-invite-link"))?
                    .to_owned(),
            ),
            None => None,
        };
        let member = actor.did.clone();
        self.require_group_enabled_agent(&member).await?;
        let _guard = self.groups.join_locks.lock(&member, group).await;
        let route = self.resolve_group_route(&member, group, true).await?;
        let mut saved = self.load_group_binding(&member, group).await?;
        if saved.as_ref().is_some_and(JoinedGroupBinding::unanswered) {
            // An earlier accept or request of this member comes first; once it is through,
            // the member is in (or waits for approval) and this request has its answer.
            let outcome = self.resume_group_binding(saved.take().unwrap()).await;
            saved = self.load_group_binding(&member, group).await?;
            match outcome {
                Ok(value) => {
                    self.forget_received_invitation(&member, group, None).await;
                    return Ok(value);
                }
                Err(error) if saved.as_ref().is_some_and(JoinedGroupBinding::unanswered) => {
                    return Err(error)
                }
                Err(_) => {}
            }
        }
        let mut binding = saved.unwrap_or_else(|| {
            JoinedGroupBinding::new(member.clone(), group.clone(), route.host.clone())
        });
        let host_changed = !binding.host.eq_ignore_ascii_case(&route.host);
        if host_changed {
            binding.member = None;
            binding.doc_cache = None;
        }
        binding.host = route.host;
        binding.pending_request = Some(PendingJoinRequest {
            idempotency_key: revision(),
            invite,
        });
        self.save_group_binding(&binding, host_changed).await?;
        let result = self.resume_group_binding(binding).await;
        if result.is_ok() {
            self.forget_received_invitation(&member, group, None).await;
        }
        result
    }

    /// Keeps a binding that still holds something (a member record or an unanswered join),
    /// deletes it otherwise.
    async fn keep_or_drop_binding(&self, binding: &JoinedGroupBinding) -> Result<()> {
        if binding.member.is_some() || binding.unanswered() {
            return self.save_group_binding(binding, false).await;
        }
        let sql = self
            .msg_box_db
            .render_sql("DELETE FROM group_joined_bindings WHERE owner=? AND group_did=?");
        sqlx::query(&sql)
            .bind(binding.owner_did.to_string())
            .bind(binding.group_did.to_string())
            .execute(self.msg_box_db.pool())
            .await
            .map_err(db_error)?;
        Ok(())
    }

    /// The host's member record answering a join of `binding`'s owner: `invitation` is the
    /// accepted invitation; for a request, a pending record must carry the request's id.
    fn check_join_reply(
        binding: &JoinedGroupBinding,
        member: Value,
        invitation: Option<&str>,
    ) -> Result<Value> {
        let _: MemberRecord = serde_json::from_value(member.clone())
            .map_err(|_| invalid("invalid-group-response"))?;
        let id_ok = match invitation {
            Some(invitation) => member["invitation_id"] == invitation,
            None => {
                member["state"] == "active"
                    || member["invitation_id"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty())
            }
        };
        if member["group_did"] != json!(binding.group_did)
            || member["member_did"] != json!(binding.owner_did)
            || !matches!(
                member["state"].as_str(),
                Some("active" | "pending_admin_approval")
            )
            || !id_ok
        {
            return Err(invalid("invalid-group-response"));
        }
        Ok(member)
    }

    async fn resume_group_binding(&self, mut binding: JoinedGroupBinding) -> Result<Value> {
        let route = binding.route(self);
        self.require_group_enabled_agent(&binding.owner_did).await?;
        if let Some(request) = binding.pending_request.clone() {
            let query: Vec<(&str, String)> = request
                .invite
                .iter()
                .map(|token| ("invite", token.clone()))
                .collect();
            let result = route
                .put_query(
                    self,
                    &format!("{}/join", route.base_path()),
                    &query,
                    &json!({"idempotency_key":request.idempotency_key}),
                )
                .await
                .and_then(|member| Self::check_join_reply(&binding, member, None));
            let member = match result {
                Ok(member) => member,
                Err(error) => {
                    // Anything but an unreachable host is the host's answer (refused,
                    // invite required, already participating, malformed): asking again
                    // with the same key would only repeat it.
                    if !matches!(error, kRPC::RPCErrors::ReasonError(_)) {
                        binding.pending_request = None;
                        self.keep_or_drop_binding(&binding).await?;
                    }
                    return Err(error);
                }
            };
            let changed = binding
                .member
                .as_ref()
                .is_none_or(|previous| previous["epoch"] != member["epoch"]);
            binding.member = Some(member);
            binding.pending_request = None;
            self.save_group_binding(&binding, changed).await?;
        } else if let Some(invitation) = binding.pending_invitation_id.clone().or_else(|| {
            binding
                .member
                .as_ref()
                .filter(|m| m["state"] == "pending_admin_approval")
                .and_then(|m| m["invitation_id"].as_str())
                .map(str::to_owned)
        }) {
            let result = route
                .put(
                    self,
                    &format!("{}/join", route.base_path()),
                    &json!({"invitation_id":invitation}),
                )
                .await;
            let member = match result {
                Ok(value) => value,
                Err(error) => {
                    if is_final_join_answer(&error) {
                        binding.pending_invitation_id = None;
                        if binding.member.as_ref().is_some_and(|member| {
                            member["state"] == "pending_admin_approval"
                                && member["invitation_id"] == invitation
                        }) {
                            binding.member = None;
                        }
                        self.keep_or_drop_binding(&binding).await?;
                    }
                    return Err(error);
                }
            };
            let member = Self::check_join_reply(&binding, member, Some(&invitation))?;
            let changed = binding
                .member
                .as_ref()
                .is_none_or(|previous| previous["epoch"] != member["epoch"]);
            binding.member = Some(member);
            binding.pending_invitation_id = None;
            self.save_group_binding(&binding, changed).await?;
        }
        let mut member = binding.member.clone().ok_or_else(missing)?;
        if member["state"] == "pending_admin_approval" {
            if binding.doc_cache.is_none() {
                binding.doc_cache = route
                    .fetch(self, &format!("{}/doc", route.base_path()), &[])
                    .await
                    .ok()
                    .filter(|doc| doc["doc"]["id"] == json!(binding.group_did));
                self.save_group_binding(&binding, false).await?;
            }
        } else {
            let sync = self.sync_joined_group(&route).await;
            member["sync_pending"] = json!(sync.is_err());
            if let Err(error) = sync {
                log::warn!(
                    "joined group {} initial sync pending: {}",
                    binding.group_did.to_string(),
                    error
                );
            }
        }
        Ok(member)
    }

    pub(crate) async fn sync_remote_group(
        &self,
        owner: &DID,
        group: &DID,
    ) -> Result<Option<Value>> {
        let _guard = self.groups.join_locks.lock(owner, group).await;
        let Some(binding) = self.load_group_binding(owner, group).await? else {
            return Ok(None);
        };
        self.resume_group_binding(binding).await.map(Some)
    }

    pub(crate) async fn scheduled_group_routes(&self) -> Result<Vec<JoinedGroupRoute>> {
        let mut routes = std::collections::BTreeMap::new();
        for route in self.cyfs_dispatch.read().unwrap().joined_groups.iter() {
            routes.insert(
                (route.owner_did.to_string(), route.group_did.to_string()),
                route.clone(),
            );
        }
        for binding in self.group_bindings(None).await? {
            let key = (binding.owner_did.to_string(), binding.group_did.to_string());
            if binding.active() {
                routes.insert(key, binding.route(self));
            } else {
                routes.remove(&key);
            }
        }
        Ok(routes.into_values().collect())
    }

    #[cfg(test)]
    pub(crate) async fn sync_remote_groups(&self) -> Result<()> {
        self.recover_remote_joins(true).await
    }

    pub(crate) async fn recover_remote_joins(&self, include_initial_sync: bool) -> Result<()> {
        let mut recovery = tokio::task::JoinSet::new();
        for binding in self.group_bindings(None).await? {
            if !binding.unanswered()
                && binding.active()
                && (!include_initial_sync
                    || self
                        .joined_state(&binding.owner_did, &binding.group_did)
                        .await?
                        .is_some())
            {
                continue;
            }
            let center = self.clone();
            recovery.spawn(async move {
                if let Err(error) = center
                    .sync_remote_group(&binding.owner_did, &binding.group_did)
                    .await
                {
                    log::warn!(
                        "joined group {} recovery failed: {error}",
                        binding.group_did.to_string()
                    );
                }
            });
        }
        while let Some(result) = recovery.join_next().await {
            if let Err(error) = result {
                log::warn!("joined group recovery task failed: {error}");
            }
        }
        Ok(())
    }

    pub(crate) async fn remote_group_invitations(&self, owner: &DID) -> Result<Vec<Value>> {
        Ok(self
            .group_bindings(Some(owner))
            .await?
            .into_iter()
            .filter_map(|binding| {
                let member = binding.member?;
                if member["state"] != "pending_admin_approval" {
                    return None;
                }
                let mut doc = binding.doc_cache.unwrap_or_else(|| json!({"doc":null}));
                doc["group_did"] = json!(binding.group_did);
                doc["hosted_here"] = json!(false);
                doc["member"] = member;
                Some(doc)
            })
            .collect())
    }

    /// The invitation a group notice carries, if it is one (`invite` of
    /// `buckyos.group_invitation`) that has not expired.
    fn received_invitation(msg: &ndn_lib::MsgObject) -> Option<ReceivedInvitation> {
        if msg.kind != ndn_lib::MsgObjKind::Operation {
            return None;
        }
        let machine = msg.content.machine.as_ref()?;
        if machine.intent.as_deref() != Some(GROUP_NOTICE_INTENT) {
            return None;
        }
        let data = serde_json::to_value(&machine.data).ok()?;
        if data["action"] != "invite" {
            return None;
        }
        let group_did: DID = serde_json::from_value(data["group_did"].clone()).ok()?;
        let detail = &data["data"];
        let invitation_id = detail["invite_id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)?
            .to_owned();
        let expires_at_ms = detail["expires_at_ms"].as_u64();
        if expires_at_ms.is_some_and(|t| t <= Self::now_ms()) {
            return None;
        }
        let profile = detail
            .get("profile")
            .filter(|p| p.is_object())
            .filter(|p| p.to_string().len() <= MAX_INVITATION_PROFILE_BYTES)
            .cloned()
            .unwrap_or(Value::Null);
        Some(ReceivedInvitation {
            schema_version: 1,
            group_did,
            invitation_id,
            invited_by: msg.from.clone(),
            member_did: detail
                .get("member_did")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            role: serde_json::from_value(detail["role"].clone()).unwrap_or(GroupRole::Member),
            expires_at_ms,
            profile,
            received_at_ms: Self::now_ms(),
        })
    }

    /// A notice from another Zone was delivered to `owner` here: when it invites them into a
    /// group hosted elsewhere, keep the invitation until they answer it, so that it is listed
    /// with their other invitations (`group.list_by_member`). The newest per group wins.
    pub(crate) async fn note_received_group_invitation(
        &self,
        owner: &DID,
        msg: &ndn_lib::MsgObject,
    ) {
        let Some(invitation) = Self::received_invitation(msg) else {
            return;
        };
        if !matches!(self.groups.load(&invitation.group_did).await, Ok(None)) {
            return;
        }
        if let Err(error) = self.store_received_invitation(owner, &invitation).await {
            log::warn!(
                "keeping invitation into {} for {} failed: {error}",
                invitation.group_did.to_string(),
                owner.to_string()
            );
        }
    }

    async fn store_received_invitation(
        &self,
        owner: &DID,
        invitation: &ReceivedInvitation,
    ) -> Result<()> {
        let sql = self
            .msg_box_db
            .render_sql("SELECT group_did FROM group_received_invitations WHERE owner=?");
        let kept: Vec<String> = sqlx::query(&sql)
            .bind(owner.to_string())
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|row| row.try_get("group_did").map_err(db_error))
            .collect::<Result<_>>()?;
        if kept.len() >= MAX_RECEIVED_INVITATIONS
            && !kept.contains(&invitation.group_did.to_string())
        {
            return Err(denied("too-many-received-invitations"));
        }
        let sql = self.msg_box_db.render_sql("INSERT INTO group_received_invitations(owner,group_did,invitation_json) VALUES(?,?,?) ON CONFLICT(owner,group_did) DO UPDATE SET invitation_json=excluded.invitation_json");
        sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(invitation.group_did.to_string())
            .bind(serde_json::to_string(invitation).map_err(db_error)?)
            .execute(self.msg_box_db.pool())
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn load_received_invitations(
        &self,
        owner: &DID,
        group: Option<&DID>,
    ) -> Result<Vec<ReceivedInvitation>> {
        let rows = match group {
            Some(group) => {
                let sql = self.msg_box_db.render_sql(
                    "SELECT invitation_json FROM group_received_invitations WHERE owner=? AND group_did=?",
                );
                sqlx::query(&sql)
                    .bind(owner.to_string())
                    .bind(group.to_string())
                    .fetch_all(self.msg_box_db.pool())
                    .await
            }
            None => {
                let sql = self.msg_box_db.render_sql(
                    "SELECT invitation_json FROM group_received_invitations WHERE owner=? ORDER BY group_did",
                );
                sqlx::query(&sql)
                    .bind(owner.to_string())
                    .fetch_all(self.msg_box_db.pool())
                    .await
            }
        }
        .map_err(db_error)?;
        rows.into_iter()
            .map(|row| {
                let raw: String = row.try_get("invitation_json").map_err(db_error)?;
                serde_json::from_str(&raw).map_err(db_error)
            })
            .collect()
    }

    /// Drops the kept invitation into `group`: any (joined or refused for good), or only
    /// `invitation` so that a newer one stays.
    pub(crate) async fn forget_received_invitation(
        &self,
        owner: &DID,
        group: &DID,
        invitation: Option<&str>,
    ) {
        let outcome = async {
            if let Some(invitation) = invitation {
                let kept = self.load_received_invitations(owner, Some(group)).await?;
                if !kept.iter().any(|kept| kept.invitation_id == invitation) {
                    return Ok(());
                }
            }
            let sql = self
                .msg_box_db
                .render_sql("DELETE FROM group_received_invitations WHERE owner=? AND group_did=?");
            sqlx::query(&sql)
                .bind(owner.to_string())
                .bind(group.to_string())
                .execute(self.msg_box_db.pool())
                .await
                .map_err(db_error)?;
            Ok::<_, kRPC::RPCErrors>(())
        }
        .await;
        if let Err(error) = outcome {
            log::warn!(
                "forgetting invitation into {} for {} failed: {error}",
                group.to_string(),
                owner.to_string()
            );
        }
    }

    /// Whether `owner` takes part in `group` (hosted elsewhere) already: joined and not ended,
    /// waiting for approval, or with an accept or request on its way.
    async fn takes_part_remotely(&self, owner: &DID, group: &DID) -> Result<bool> {
        if matches!(self.joined_group_route(owner, group).await, Ok(Some(_))) {
            return Ok(true);
        }
        Ok(self
            .load_group_binding(owner, group)
            .await?
            .is_some_and(|binding| binding.unanswered() || binding.pending_approval()))
    }

    /// The invitations kept for `owner` that still wait for an answer, shaped like the
    /// `invitations` of `group.list_by_member`; they carry no group document.
    pub(crate) async fn received_group_invitations(&self, owner: &DID) -> Result<Vec<Value>> {
        let now = Self::now_ms();
        let mut out = vec![];
        for invitation in self.load_received_invitations(owner, None).await? {
            let group = &invitation.group_did;
            if invitation.expires_at_ms.is_some_and(|t| t <= now) {
                self.forget_received_invitation(owner, group, Some(&invitation.invitation_id))
                    .await;
                continue;
            }
            if self.groups.load(group).await?.is_some()
                || self.takes_part_remotely(owner, group).await?
            {
                continue;
            }
            let mut member = json!({
                "state":"invited",
                "role":invitation.role,
                "invitation_id":invitation.invitation_id,
                "invited_by":invitation.invited_by,
                "expires_at_ms":invitation.expires_at_ms,
            });
            if let Some(did) = &invitation.member_did {
                member["member_did"] = json!(did);
            }
            out.push(json!({
                "doc":null,
                "group_did":group,
                "hosted_here":false,
                "profile":invitation.profile,
                "received_at_ms":invitation.received_at_ms,
                "member":member,
            }));
        }
        Ok(out)
    }

    /// `owner`'s state in a group hosted elsewhere, as `contact.lookup` reports it.
    pub(crate) async fn remote_group_state(
        &self,
        owner: &DID,
        group: &DID,
    ) -> Result<Option<&'static str>> {
        if matches!(self.joined_group_route(owner, group).await, Ok(Some(_))) {
            return Ok(Some("active"));
        }
        if let Some(binding) = self.load_group_binding(owner, group).await? {
            if binding.pending_approval() {
                return Ok(Some("pending_admin_approval"));
            }
        }
        let now = Self::now_ms();
        Ok(self
            .load_received_invitations(owner, Some(group))
            .await?
            .iter()
            .any(|invitation| invitation.expires_at_ms.is_none_or(|t| t > now))
            .then_some("invited"))
    }
}
