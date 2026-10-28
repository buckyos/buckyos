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
    host: String,
    pending_invitation_id: Option<String>,
    member: Option<Value>,
    doc_cache: Option<Value>,
}

impl JoinedGroupBinding {
    fn route(&self, center: &MessageCenter) -> JoinedGroupRoute {
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
    ) -> Result<JoinedGroupRoute> {
        let profile = self
            .directory
            .get()
            .describe(group)
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

    async fn load_group_binding(
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
        let _guard = self.groups.sync_lock.lock().await;
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
                state.session_cache.clear();
                let sql = self.msg_box_db.render_sql(
                    "UPDATE joined_groups SET state_json=? WHERE owner=? AND group_did=?",
                );
                sqlx::query(&sql)
                    .bind(serde_json::to_string(&state).map_err(db_error)?)
                    .bind(binding.owner_did.to_string())
                    .bind(binding.group_did.to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(db_error)?;
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
            if binding
                .member
                .as_ref()
                .is_some_and(|member| member["state"] == "active")
            {
                if self
                    .joined_groups(owner)
                    .await?
                    .iter()
                    .any(|state| state.group_did == *group && state.stopped)
                {
                    return Err(denied("group-membership-ended"));
                }
                return Ok(Some(binding.route(self)));
            }
            return Err(denied("member-not-active"));
        }
        Ok(self
            .cyfs_dispatch
            .read()
            .unwrap()
            .joined_groups
            .iter()
            .find(|route| route.owner_did == *owner && route.group_did == *group)
            .cloned())
    }

    pub(crate) async fn remote_group_doc(&self, owner: &DID, group: &DID) -> Result<Value> {
        let route = self.resolve_group_route(owner, group).await?;
        let value = route
            .fetch(self, &format!("{}/doc", route.base_path()), &[])
            .await?;
        if value["doc"]["id"] != json!(group) {
            return Err(invalid("invalid-group-response"));
        }
        Ok(value)
    }

    pub(crate) async fn accept_remote_group_invitation(
        &self,
        group: &DID,
        actor: &GroupActor,
        p: &Value,
        ctx: &RPCContext,
    ) -> Result<Value> {
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
        let _guard = self.groups.join_lock.lock().await;
        let route = self.resolve_group_route(&member, group).await?;
        let mut saved = self.load_group_binding(&member, group).await?;
        if saved.as_ref().is_some_and(|binding| {
            binding
                .pending_invitation_id
                .as_deref()
                .is_some_and(|pending| pending != invitation)
        }) {
            let outcome = self.resume_group_binding(saved.take().unwrap()).await;
            saved = self.load_group_binding(&member, group).await?;
            if let Err(error) = outcome {
                if saved
                    .as_ref()
                    .is_some_and(|binding| binding.pending_invitation_id.is_some())
                {
                    return Err(error);
                }
            }
        }
        let mut binding = saved.unwrap_or(JoinedGroupBinding {
            schema_version: 1,
            owner_did: member,
            group_did: group.clone(),
            host: route.host.clone(),
            pending_invitation_id: None,
            member: None,
            doc_cache: None,
        });
        binding.host = route.host;
        binding.pending_invitation_id = Some(invitation.into());
        self.save_group_binding(&binding, false).await?;
        self.resume_group_binding(binding).await
    }

    async fn resume_group_binding(&self, mut binding: JoinedGroupBinding) -> Result<Value> {
        let route = binding.route(self);
        self.require_group_enabled_agent(&binding.owner_did).await?;
        let invitation = binding.pending_invitation_id.clone().or_else(|| {
            binding
                .member
                .as_ref()
                .filter(|m| m["state"] == "pending_admin_approval")
                .and_then(|m| m["invitation_id"].as_str())
                .map(str::to_owned)
        });
        if let Some(invitation) = invitation {
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
                    if matches!(&error, kRPC::RPCErrors::NoPermission(reason) if matches!(reason.as_str(),
                        "invitation-mismatch" | "invitation-expired" | "invitation-revoked" | "not-found" | "remote-group-not-found" | "blocked" | "client-not-allowed"))
                    {
                        binding.pending_invitation_id = None;
                        if binding.member.as_ref().is_some_and(|member| {
                            member["state"] == "pending_admin_approval"
                                && member["invitation_id"] == invitation
                        }) {
                            binding.member = None;
                        }
                        if binding.member.is_some() {
                            self.save_group_binding(&binding, false).await?;
                        } else {
                            let sql = self.msg_box_db.render_sql(
                                "DELETE FROM group_joined_bindings WHERE owner=? AND group_did=?",
                            );
                            sqlx::query(&sql)
                                .bind(binding.owner_did.to_string())
                                .bind(binding.group_did.to_string())
                                .execute(self.msg_box_db.pool())
                                .await
                                .map_err(db_error)?;
                        }
                    }
                    return Err(error);
                }
            };
            let _: MemberRecord = serde_json::from_value(member.clone())
                .map_err(|_| invalid("invalid-group-response"))?;
            if member["group_did"] != json!(binding.group_did)
                || member["member_did"] != json!(binding.owner_did)
                || !matches!(
                    member["state"].as_str(),
                    Some("active" | "pending_admin_approval")
                )
                || member["invitation_id"] != invitation
            {
                return Err(invalid("invalid-group-response"));
            }
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
        let _guard = self.groups.join_lock.lock().await;
        let Some(binding) = self.load_group_binding(owner, group).await? else {
            return Ok(None);
        };
        self.resume_group_binding(binding).await.map(Some)
    }

    pub(crate) async fn sync_remote_groups(&self) -> Result<()> {
        for binding in self.group_bindings(None).await? {
            if let Err(error) = self
                .sync_remote_group(&binding.owner_did, &binding.group_did)
                .await
            {
                log::warn!(
                    "joined group {} recovery failed: {}",
                    binding.group_did.to_string(),
                    error
                );
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
}
