use crate::group_store::{db_error, GroupTransaction};
use crate::group_types::*;
use crate::msg_center::MessageCenter;
use buckyos_api::{
    get_buckyos_api_runtime, DeliveryRecord, DeliveryState, DispatchResult, MailboxAddress,
    MailboxKind, MailboxRecord, RecipientState, TokenPrincipalKind,
};
use kRPC::RPCContext;
use name_lib::DID;
use ndn_lib::{
    CanonValue, MachineContent, MsgContent, MsgContentFormat, MsgObjKind, MsgObject, MsgRelType,
    NamedObject, ObjId,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn field<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Result<T> {
    serde_json::from_value(
        v.get(k)
            .cloned()
            .ok_or_else(|| invalid(format!("missing-{k}")))?,
    )
    .map_err(|e| invalid(format!("{k}:{e}")))
}
fn optional<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Result<Option<T>> {
    v.get(k)
        .filter(|v| !v.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| invalid(format!("{k}:{e}")))
}
fn encoded<T: serde::Serialize>(v: T) -> Result<Value> {
    serde_json::to_value(v).map_err(db_error)
}
fn expected(v: &Value, current: &str) -> Result<()> {
    if field::<String>(v, "expected_revision")? != current {
        return Err(kRPC::RPCErrors::ReasonError(format!(
            "revision-conflict:{current}"
        )));
    }
    Ok(())
}
fn within(window: Option<u64>, start: u64, now: u64) -> bool {
    window.is_none_or(|w| w > 0 && now.saturating_sub(start) <= w)
}
const JOIN_METHODS: &[&str] = &[
    "group.accept_invitation",
    "group.request_join",
    "group.accept_session_invitation",
    "group.submit_guest_request",
];
/// How the invited member's own Zone answers an invitation on their behalf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemberConsent {
    /// The member's Zone accepts immediately (friend or agent owner).
    Accept,
    /// Deliver the invitation and wait for an explicit acceptance.
    Ask,
    /// The inviter is blocked: nothing is delivered.
    Drop,
}

impl MessageCenter {
    pub(crate) async fn group_actor(&self, ctx: &RPCContext) -> Result<GroupActor> {
        let caller = self
            .caller_identity(ctx)
            .await?
            .ok_or_else(|| denied("authentication-required"))?;
        let verified = self
            .token_verifier
            .get()
            .verify(ctx.token.as_deref().unwrap())
            .await?;
        let did = match caller.user_did {
            Some(d) => d,
            None => {
                let config = self.cyfs_dispatch.read().unwrap().clone();
                config
                    .principal_dids
                    .get(&caller.user_id)
                    .cloned()
                    .or_else(|| DID::from_str(&caller.user_id).ok())
                    .ok_or_else(|| denied("principal-did-unavailable"))?
            }
        };
        if caller.principal_kind == TokenPrincipalKind::App && verified.appid.is_none() {
            return Err(denied("client-identity-required"));
        }
        Ok(GroupActor {
            did,
            client: verified.appid,
            remote: false,
        })
    }

    pub(crate) async fn group_rpc(&self, method: &str, p: Value, ctx: RPCContext) -> Result<Value> {
        if p.get("actor_did").is_some() || p.get("host_owner").is_some() {
            return Err(invalid("actor-is-derived-from-authentication"));
        }
        let actor = self.group_actor(&ctx).await?;
        self.group_rpc_authenticated(method, p, ctx, actor).await
    }
    pub(crate) async fn group_message_actor(
        &self,
        ctx: &RPCContext,
        msg: &MsgObject,
    ) -> Result<GroupActor> {
        let mut actor = self.group_actor(ctx).await?;
        if let Some((_, _, instance)) =
            crate::contact_mgr::ContactMgr::parse_msgtunnel_did(&msg.from)
        {
            if self
                .lookup_tunnel_route(&instance)
                .is_some_and(|r| r.transport_did == actor.did)
            {
                actor.did = msg.from.clone();
            }
        }
        if actor.did != msg.from {
            return Err(denied("sender-mismatch"));
        }
        Ok(actor)
    }

    pub(crate) async fn group_rpc_authenticated(
        &self,
        method: &str,
        p: Value,
        ctx: RPCContext,
        mut actor: GroupActor,
    ) -> Result<Value> {
        let authenticated_did = actor.did.clone();
        if p.get("actor_did").is_some() || p.get("host_owner").is_some() {
            return Err(invalid("actor-is-derived-from-authentication"));
        }
        if method == "group.create" {
            self.authorize_resource(&ctx, "obj://msg-center/group", "create")
                .await?;
            return self.create_group(&actor, p).await;
        }
        if method == "group.list_by_member" {
            let mut groups = vec![];
            for g in self.groups.list().await? {
                if g.lifecycle != "deleted"
                    && (g.role(&actor.did).is_some()
                        || g.sessions.keys().any(|s| g.effective(Some(s), &actor.did)))
                    && g.check_client(&actor).is_ok()
                {
                    groups.push(self.group_doc(&g)?);
                }
            }
            let joined = self.joined_groups(&actor.did).await?;
            return Ok(json!({"items":groups,"joined":joined}));
        }
        let group: DID = field(&p, "group_did")?;
        if method == "group.sync_joined" {
            let route = self
                .cyfs_dispatch
                .read()
                .unwrap()
                .joined_groups
                .iter()
                .find(|r| r.owner_did == actor.did && r.group_did == group)
                .cloned()
                .ok_or_else(missing)?;
            return self.sync_joined_group(&route).await;
        }
        match method {
            "group.get_doc" => {
                let g = self
                    .groups
                    .load(&group)
                    .await?
                    .filter(|g| g.lifecycle != "deleted")
                    .ok_or_else(missing)?;
                return self.group_doc(&g);
            }
            "group.get_config" => {
                let g = self.groups.load(&group).await?.ok_or_else(missing)?;
                g.require_cap(&actor, "session.read", None)?;
                return Ok(g.configuration);
            }
            "group.list_sessions" => return self.group_sessions(&actor, &group).await,
            "group.list_session_members" => {
                return self
                    .group_session_members(
                        &actor,
                        &group,
                        optional::<String>(&p, "session_id")?.as_deref(),
                    )
                    .await
            }
            "group.list_messages" => {
                return self
                    .group_inbox(
                        &actor,
                        &group,
                        optional::<String>(&p, "session_id")?.as_deref(),
                        optional(&p, "after_seq")?.unwrap_or(0),
                        optional(&p, "limit")?.unwrap_or(100),
                    )
                    .await
            }
            "group.changes" => {
                return self
                    .group_changes(
                        &actor,
                        &group,
                        optional::<String>(&p, "since")?.as_deref(),
                        optional(&p, "limit")?.unwrap_or(100),
                    )
                    .await
            }
            "group.check_access" => {
                let g = self.groups.load(&group).await?.ok_or_else(missing)?;
                let s: Option<String> = optional(&p, "session_id")?;
                let action: String = field(&p, "action")?;
                let result = if matches!(action.as_str(), "session.read" | "session.post") {
                    match g.require_session(&actor, s.as_deref(), action == "session.read") {
                        Err(e) => Err(e),
                        Ok(())
                            if action == "session.post"
                                && (g.lifecycle != "active"
                                    || s.as_ref()
                                        .is_some_and(|s| g.sessions[s].lifecycle != "active")) =>
                        {
                            Err(denied("session-archived"))
                        }
                        Ok(())
                            if action == "session.post"
                                && !g.native_post_allowed(&actor, s.as_deref(), false)? =>
                        {
                            Err(denied("post-not-allowed"))
                        }
                        Ok(())
                            if action == "session.read"
                                && !self
                                    .run_group_hooks(&g, &actor, s.as_deref(), "read", true, None)
                                    .await? =>
                        {
                            Err(denied("read-not-allowed"))
                        }
                        Ok(()) => Ok(()),
                    }
                } else {
                    g.require_cap(&actor, &action, s.as_deref())
                };
                return Ok(
                    json!({"allowed":result.is_ok(),"reason":result.err().map(|e|e.to_string())}),
                );
            }
            "group.list_members" => {
                self.expire_group_invitations(&group).await?;
                let g = self.groups.load(&group).await?.ok_or_else(missing)?;
                if g.role(&actor.did).is_none() || g.blocked(&actor.did) || g.lifecycle == "deleted"
                {
                    return Err(missing());
                }
                g.check_client(&actor)?;
                if g.config()?.membership.member_list_visibility == "admins_only"
                    && !g.capability(&actor.did, "group.approve_member")
                {
                    return Err(denied("member-list-hidden"));
                }
                return Ok(
                    json!({"items":g.members.values().collect::<Vec<_>>(),"pending_owner_transfer":g.pending_owner_transfer.as_ref().filter(|t|t.expires_at_ms>Self::now_ms())}),
                );
            }
            "group.list_events" => {
                let g = self.groups.load(&group).await?.ok_or_else(missing)?;
                g.require_cap(&actor, "group.read_all", None)?;
                let after = optional::<u64>(&p, "after_seq")?.unwrap_or(0);
                let limit = optional::<usize>(&p, "limit")?
                    .unwrap_or(100)
                    .clamp(1, 4096);
                return Ok(
                    json!({"items":g.changes.iter().filter(|c|c.group_seq>after).take(limit).collect::<Vec<_>>(),"audit":g.audit}),
                );
            }
            _ => {}
        }
        if let Some(key) = optional::<String>(&p, "idempotency_key")? {
            if let Some(saved) = self.groups.load(&group).await?.and_then(|g| {
                g.operations
                    .get(&format!("{}|{}|{}", actor.did.to_string(), method, key))
                    .cloned()
            }) {
                if saved.request != p {
                    return Err(invalid("idempotency-key-reused"));
                }
                return Ok(saved.result);
            }
        }
        let mut attestation = None;
        if JOIN_METHODS.contains(&method) {
            if let Some(input) = p.get("attestation") {
                let verified = self.verify_tunnel_attestation(&actor, input)?;
                actor.did = field(&verified, "member_did")?;
                attestation = Some(verified);
            }
        }
        if method == "group.accept_invitation" {
            if let Some(member) = optional::<DID>(&p, "member_did")? {
                if member != actor.did {
                    if attestation.is_some() {
                        return Err(invalid("member-did-conflicts-with-attestation"));
                    }
                    let owner = self.token_verifier.get().agent_owner(&member).await?;
                    if owner.as_ref() != Some(&authenticated_did) {
                        return Err(denied("agent-owner-required"));
                    }
                    actor.did = member;
                }
            }
        }
        let mut tx = self.groups.begin(&group).await?;
        if tx.state.lifecycle == "deleted" {
            return Err(missing());
        }
        let key = optional::<String>(&p, "idempotency_key")?;
        if matches!(
            method,
            "group.apply_config" | "group.update_shared_state" | "group.update_member_state"
        ) && key.is_none()
        {
            return Err(invalid("idempotency-key-required"));
        }
        let op_key = key
            .as_ref()
            .map(|key| format!("{}|{}|{}", authenticated_did.to_string(), method, key));
        if let Some(saved) = op_key.as_ref().and_then(|k| tx.state.operations.get(k)) {
            if saved.request != p {
                return Err(invalid("idempotency-key-reused"));
            }
            return Ok(saved.result.clone());
        }
        let result = self
            .mutate_group(&mut tx, &actor, method, &p, attestation)
            .await?;
        if let Some(k) = op_key {
            tx.state.operations.insert(
                k,
                SavedOperation {
                    request: p,
                    result: result.clone(),
                },
            );
        }
        tx.commit().await?;
        Self::publish_event(
            format!("/msg_center/group/{}/changed", group.to_string()),
            json!({"group_did":group}),
        );
        Ok(result)
    }

    pub(crate) fn group_doc(&self, g: &GroupState) -> Result<Value> {
        let doc = g.public_doc()?;
        let (id, _) = ndn_lib::build_named_object_by_json("group", &doc);
        Ok(json!({"obj_id":id,"doc":doc}))
    }
    pub(crate) async fn expire_group_invitations(&self, group: &DID) -> Result<()> {
        let Some(g) = self.groups.load(group).await? else {
            return Ok(());
        };
        let now = Self::now_ms();
        let expired = |m: &&MemberRecord| {
            m.state == MemberStatus::Invited && m.expires_at_ms.is_some_and(|t| t <= now)
        };
        if !g.members.values().any(|m| expired(&m)) {
            return Ok(());
        }
        let mut tx = self.groups.begin(group).await?;
        let members: Vec<_> = tx
            .state
            .members
            .values()
            .filter(expired)
            .map(|m| m.member_did.clone())
            .collect();
        let system = GroupActor {
            did: group.clone(),
            client: None,
            remote: false,
        };
        for member in members {
            tx.state.members.get_mut(&member.to_string()).unwrap().state = MemberStatus::Expired;
            self.group_event(
                &mut tx,
                &system,
                None,
                "entity.invite_expired",
                Some(member),
                json!([]),
            )
            .await?;
        }
        tx.commit().await
    }

    /// External platform users (`did:msgtunnel:*`) cannot act for themselves;
    /// the registered tunnel transport submits their consent as an attestation
    /// carrying the platform-side source event.
    fn verify_tunnel_attestation(&self, a: &GroupActor, input: &Value) -> Result<Value> {
        let member: DID = field(input, "member_did")?;
        if member.method != "msgtunnel" {
            return Err(denied("invalid-tunnel-attestation"));
        }
        let (_, _, instance) = crate::contact_mgr::ContactMgr::parse_msgtunnel_did(&member)
            .ok_or_else(|| denied("invalid-shadow-endpoint"))?;
        let route = self
            .lookup_tunnel_route(&instance)
            .ok_or_else(|| denied("unknown-tunnel"))?;
        if a.remote
            || route.transport_did != a.did
            || !input["source_event"].is_object()
            || input["source_event"]["event_id"]
                .as_str()
                .is_none_or(|s| s.is_empty())
            || input["source_event"]["user_consent"] != true
        {
            return Err(denied("invalid-tunnel-attestation"));
        }
        Ok(json!({"member_did":member,"attested_by":a.did,"source_event":input["source_event"]}))
    }

    /// Entity kind of a member (`user`, `agent`, `device`), taken from its DID
    /// Document. Shadow endpoints are platform users and `did:dev` keys are
    /// devices; a document that cannot be fetched right now yields `unknown`
    /// and is retried when the member becomes active.
    async fn resolve_entity_kind(&self, d: &DID) -> Result<String> {
        if d.method == "msgtunnel" {
            return Ok("user".into());
        }
        if self.is_local_recipient(d)
            && self
                .token_verifier
                .get()
                .is_zone_agent(d)
                .await
                .unwrap_or(false)
        {
            return Ok("agent".into());
        }
        if d.method == "dev" {
            return Ok("device".into());
        }
        let doc = match name_client::resolve_did(d, None).await {
            Ok(doc) => doc.to_json_value().map_err(db_error)?,
            Err(_) => return Ok("unknown".into()),
        };
        let context: Option<name_lib::DIDContext> = doc
            .get("@context")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(db_error)?;
        let declared = doc
            .get("entity_type")
            .or_else(|| doc.get("type"))
            .or_else(|| doc.get("doc_type"))
            .and_then(Value::as_str);
        let kind = match declared {
            Some("user" | "owner") => "user",
            Some("agent") => "agent",
            Some("device") => "device",
            Some(_) => return Err(denied("member-must-be-single-entity")),
            None if context
                .as_ref()
                .is_some_and(|c| c.contains("https://buckyos.org/ns/owner/v1")) =>
            {
                "user"
            }
            None if context
                .as_ref()
                .is_some_and(|c| c.contains("https://buckyos.org/ns/agent/v1")) =>
            {
                "agent"
            }
            None if context
                .as_ref()
                .is_some_and(|c| c.contains("https://buckyos.org/ns/device/v1")) =>
            {
                "device"
            }
            _ => return Err(denied("member-must-be-single-entity")),
        };
        if doc["id"] != json!(d) {
            return Err(denied("member-document-mismatch"));
        }
        Ok(kind.to_string())
    }

    /// The invited member's side of the consent: same-Zone users answer by
    /// their Contact Mgr policy towards the inviter, agents only accept their
    /// owner's invitations (anyone else's go to the owner for confirmation),
    /// and remote members still confirm explicitly (cross-Zone auto-accept is
    /// a TODO). Returns the decision and who receives the invitation notice.
    async fn member_consent(&self, inviter: &DID, d: &DID) -> Result<(MemberConsent, DID)> {
        if d.method == "msgtunnel" || !self.is_local_recipient(d) {
            return Ok((MemberConsent::Ask, d.clone()));
        }
        if let Some(owner) = self.token_verifier.get().agent_owner(d).await? {
            return Ok(if owner == *inviter {
                (MemberConsent::Accept, d.clone())
            } else {
                (MemberConsent::Ask, owner)
            });
        }
        let access = self
            .contact_mgr
            .peek_access_permission(inviter, None, Some(d))
            .await?;
        Ok(match access.target_box.as_str() {
            "INBOX" => (MemberConsent::Accept, d.clone()),
            "DROP" => (MemberConsent::Drop, d.clone()),
            _ => (MemberConsent::Ask, d.clone()),
        })
    }

    async fn create_group(&self, a: &GroupActor, p: Value) -> Result<Value> {
        let key: String = field(&p, "idempotency_key")?;
        if key.is_empty() {
            return Err(invalid("idempotency-key-required"));
        }
        let mut tx = self.groups.db.pool().begin().await.map_err(db_error)?;
        sqlx::query("UPDATE group_locks SET schema_version=schema_version WHERE lock_key='create'")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let sql=self.groups.db.render_sql("SELECT request_json,result_json FROM group_create_operations WHERE actor=? AND operation_key=?");
        if let Some(r) = sqlx::query(&sql)
            .bind(a.did.to_string())
            .bind(&key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?
        {
            use sqlx::Row;
            let request: String = r.try_get("request_json").map_err(db_error)?;
            if serde_json::from_str::<Value>(&request).map_err(db_error)? != p {
                return Err(invalid("idempotency-key-reused"));
            }
            let result: String = r.try_get("result_json").map_err(db_error)?;
            return serde_json::from_str(&result).map_err(db_error);
        }
        let host = match get_buckyos_api_runtime() {
            Ok(r) => r.zone_id.clone(),
            Err(_) => self
                .cyfs_dispatch
                .read()
                .unwrap()
                .target_zone
                .as_ref()
                .map(|s| DID::new("web", s))
                .ok_or_else(|| invalid("host-zone-unavailable"))?,
        };
        let chosen = optional::<String>(&p, "group_id")?;
        if let Some(id) = &chosen {
            name_lib::validate_zone_child_label(id).map_err(invalid)?;
        }
        let did = match optional::<DID>(&p, "group_did")? {
            Some(did) => did,
            None => name_lib::zone_child_did(
                &host,
                &chosen.unwrap_or_else(|| format!("g{}", uuid::Uuid::new_v4().simple())),
            )
            .map_err(invalid)?,
        };
        MailboxAddress::new(did.clone(), None).map_err(invalid)?;
        let controller = if did.method == host.method && did.id.ends_with(&format!(".{}", host.id))
        {
            let label = did.id.strip_suffix(&format!(".{}", host.id)).unwrap();
            if name_lib::zone_child_did(&host, label).map_err(invalid)? != did {
                return Err(invalid("invalid-group-id"));
            }
            a.did.clone()
        } else {
            let doc = name_client::resolve_did(&did, Some(name_lib::DidDocType::custom("group")))
                .await
                .map_err(|e| {
                    kRPC::RPCErrors::ReasonError(format!("group-document-unavailable:{e}"))
                })?
                .to_json_value()
                .map_err(db_error)?;
            let controller: DID = field(&doc, "controller")?;
            if controller != a.did
                || doc["host"] != json!(host)
                || doc["id"] != json!(did)
                || doc["entity_type"] != "group"
            {
                return Err(denied("group-did-requires-host-controller-authorization"));
            }
            controller
        };
        let entity_kind = self.resolve_entity_kind(&a.did).await?;
        let mut config: Value = p
            .get("configuration")
            .cloned()
            .unwrap_or(encoded(GroupConfiguration::default())?);
        if let Some(profile) = p.get("profile") {
            config["profile"] = profile.clone();
        }
        let mut c: GroupConfiguration =
            serde_json::from_value(config).map_err(|e| invalid(e.to_string()))?;
        c.revision = revision();
        c.validate()?;
        // Persist the fully populated form: a later partial patch of a map
        // such as `roles` must not wipe the defaults of the other keys.
        let config = encoded(&c)?;
        let mut state = GroupState {
            schema_version: 1,
            group_did: did.clone(),
            controller,
            owner: a.did.clone(),
            host,
            lifecycle: "active".into(),
            config_history: BTreeMap::from([(
                config["revision"].as_str().unwrap().to_owned(),
                config.clone(),
            )]),
            configuration: config,
            doc_updated_at_ms: Self::now_ms(),
            members: BTreeMap::new(),
            sessions: BTreeMap::new(),
            participants: BTreeMap::new(),
            intervals: BTreeMap::new(),
            moderation: BTreeMap::new(),
            pending_owner_transfer: None,
            invite_links: BTreeMap::new(),
            group_seq: 0,
            session_seqs: BTreeMap::new(),
            accepted_at_ms: 0,
            messages: BTreeMap::new(),
            changes: vec![],
            shared_states: BTreeMap::new(),
            member_states: BTreeMap::new(),
            operations: BTreeMap::new(),
            read_markers: BTreeMap::new(),
            rates: BTreeMap::new(),
            audit: vec![],
            tombstone_readers: BTreeSet::new(),
        };
        state.check_client(a)?;
        state.members.insert(
            a.did.to_string(),
            MemberRecord {
                member_did: a.did.clone(),
                role: GroupRole::Owner,
                state: MemberStatus::Active,
                epoch: 1,
                entity_kind,
                invitation_id: None,
                invited_by: None,
                expires_at_ms: None,
                since_seq: 1,
            },
        );
        let sql=self.groups.db.render_sql("INSERT INTO group_states(group_did,state_json) VALUES(?,?) ON CONFLICT(group_did) DO NOTHING");
        if sqlx::query(&sql)
            .bind(did.to_string())
            .bind(serde_json::to_string(&state).map_err(db_error)?)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?
            .rows_affected()
            == 0
        {
            return Err(invalid("group-did-already-used"));
        }
        let mut tx = GroupTransaction {
            tx,
            state,
            store: self.groups.clone(),
            projections: vec![],
        };
        self.group_event(&mut tx, a, None, "entity.group_created", None, json!({}))
            .await?;
        for s in optional::<Vec<Value>>(&p, "sessions")?.unwrap_or_default() {
            self.create_group_session(&mut tx, a, &s, false).await?;
        }
        for i in optional::<Vec<Value>>(&p, "invitations")?.unwrap_or_default() {
            self.invite_group_member(&mut tx, a, &i).await?;
        }
        let result = json!({"group_did":did,"revision":c.revision});
        let sql=self.groups.db.render_sql("INSERT INTO group_create_operations(actor,operation_key,request_json,result_json) VALUES(?,?,?,?)");
        sqlx::query(&sql)
            .bind(a.did.to_string())
            .bind(key)
            .bind(p.to_string())
            .bind(result.to_string())
            .execute(&mut *tx.tx)
            .await
            .map_err(db_error)?;
        tx.commit().await?;
        Self::publish_event(
            format!("/msg_center/group/{}/changed", did.to_string()),
            json!({"group_did":did}),
        );
        self.register_local_recipients([did]);
        Ok(result)
    }

    async fn run_group_hooks(
        &self,
        g: &GroupState,
        a: &GroupActor,
        s: Option<&str>,
        point: &str,
        native: bool,
        msg: Option<&MsgObject>,
    ) -> Result<bool> {
        let rules = g.rules(s)?;
        let mut granted = native;
        for h in rules.hooks.iter().filter(|h| h.point == point) {
            let cache_key = format!(
                "{}|{}|{}|{}|{}|{}|{:?}|{}",
                g.group_did.to_string(),
                s.unwrap_or(""),
                a.did.to_string(),
                g.config()?.revision,
                s.map(|s| g.sessions[s].revision.as_str()).unwrap_or(""),
                h.service,
                a.client,
                a.remote
            );
            if point == "read"
                && native
                && self
                    .groups
                    .read_hook_cache
                    .lock()
                    .unwrap()
                    .get(&cache_key)
                    .is_some_and(|until| *until > std::time::Instant::now())
            {
                continue;
            }
            let configured = self
                .cyfs_dispatch
                .read()
                .unwrap()
                .group_hook_authorization
                .clone();
            let token = match get_buckyos_api_runtime() {
                Ok(r) => Some(r.session_token.read().await.clone()).filter(|s| !s.is_empty()),
                Err(_) => None,
            }
            .map(|s| format!("Bearer {s}"))
            .or(configured);
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(db_error)?;
            let mut call=client.post(&h.service).json(&json!({"operation":point,"actor":a.did,"group_did":g.group_did,"session_id":s,"msg":msg}));
            if let Some(t) = token {
                call = call.header("authorization", t);
            } else {
                if h.on_unavailable == "deny" {
                    return Err(kRPC::RPCErrors::ReasonError("hook-unavailable".into()));
                }
                continue;
            }
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(h.timeout_ms as u64),
                async {
                    let response = call.send().await?;
                    response.error_for_status()?.json::<Value>().await
                },
            )
            .await;
            match result
                .ok()
                .and_then(|r| r.ok())
                .and_then(|v| v.get("decision").and_then(Value::as_str).map(str::to_owned))
                .as_deref()
            {
                Some("allow") => {
                    if native || (point == "post" && h.may_grant) {
                        granted = true;
                        if point == "read" && native && h.result_ttl_ms > 0 {
                            let mut cache = self.groups.read_hook_cache.lock().unwrap();
                            cache.retain(|_, until| *until > std::time::Instant::now());
                            cache.insert(
                                cache_key,
                                std::time::Instant::now()
                                    + std::time::Duration::from_millis(h.result_ttl_ms as u64),
                            );
                        }
                    }
                }
                Some("deny") => return Err(denied("hook-denied")),
                _ if h.on_unavailable == "deny" => {
                    return Err(kRPC::RPCErrors::ReasonError("hook-unavailable".into()))
                }
                _ => {
                    granted = native;
                }
            }
        }
        Ok(granted)
    }

    fn allocate(g: &mut GroupState, s: Option<&str>) -> (u64, u64, u64) {
        g.group_seq += 1;
        g.accepted_at_ms = g.accepted_at_ms.max(Self::now_ms());
        let seq = g.session_seqs.entry(s.unwrap_or("").into()).or_default();
        *seq += 1;
        (g.group_seq, *seq, g.accepted_at_ms)
    }
    async fn append_group_message(
        &self,
        tx: &mut GroupTransaction<'_>,
        msg: &MsgObject,
        jwt: Option<&str>,
        seq: (u64, u64, u64),
        change_kind: &str,
        subject: Option<DID>,
        data: Value,
        additional: BTreeSet<String>,
    ) -> Result<MessageMeta> {
        let (group_seq, session_seq, accepted_at_ms) = seq;
        let s = msg.to_session.as_deref();
        let id = msg.gen_obj_id().0;
        let meta = MessageMeta {
            obj_id: id.clone(),
            session_id: msg.to_session.clone(),
            group_seq,
            session_seq,
            accepted_at_ms,
            from: msg.from.clone(),
            kind: msg.kind,
            relation: msg
                .relates_to
                .as_ref()
                .map(|r| (r.rel.clone(), r.target.clone(), r.key.clone())),
            redacted: false,
        };
        tx.object(msg, jwt).await?;
        tx.state.messages.insert(id.to_string(), meta.clone());
        let mut audience = tx.state.audience(s);
        audience.extend(additional);
        tx.state.changes.push(Change {group_seq,kind:change_kind.into(),session_id:msg.to_session.clone(),subject,data:json!({"message":{"seq":session_seq,"obj_id":id,"redacted":false,"accepted_at_ms":accepted_at_ms},"change":data,"action":msg.content.content}),audience});
        let group = tx.state.group_did.clone();
        let mut record = Self::build_mailbox_record(
            group.clone(),
            MailboxKind::GroupInbox,
            msg,
            RecipientState::Unread,
            None,
            vec![format!("session_seq:{session_seq}")],
            "group-inbox",
        )?;
        record.session_id = msg.to_session.clone();
        record.mailbox =
            MailboxAddress::new(group.clone(), msg.to_session.clone()).map_err(invalid)?;
        record.sort_key = accepted_at_ms;
        tx.mailbox(&record, msg).await?;
        if msg.kind == MsgObjKind::GroupMsg && self.is_local_recipient(&msg.from) {
            let mut sent = Self::build_mailbox_record(
                msg.from.clone(),
                MailboxKind::Sent,
                msg,
                RecipientState::Read,
                None,
                vec![
                    format!("group:{}", group.to_string()),
                    format!("session_seq:{session_seq}"),
                ],
                "owner-sent",
            )?;
            sent.sort_key = accepted_at_ms;
            tx.mailbox(&sent, msg).await?;
        }
        let local_key = session_key(&group, s)?;
        for d in tx.state.audience(s) {
            let did = DID::from_str(&d).map_err(invalid)?;
            let actor = GroupActor {
                did: did.clone(),
                client: None,
                remote: false,
            };
            if !self
                .run_group_hooks(&tx.state, &actor, s, "read", true, None)
                .await
                .unwrap_or(false)
            {
                continue;
            }
            if did.method == "msgtunnel" {
                let envelope = self.build_delivery_envelope(&id, did).map_err(db_error)?;
                let record = DeliveryRecord {
                    delivery_id: Self::build_delivery_id(
                        &id,
                        &envelope.target_did,
                        &envelope.transport_did,
                    ),
                    envelope,
                    state: DeliveryState::Wait,
                    attempts: 0,
                    next_retry_at_ms: None,
                    external_msg_id: None,
                    delivered_at_ms: None,
                    last_error: None,
                    created_at_ms: accepted_at_ms,
                    updated_at_ms: accepted_at_ms,
                };
                tx.delivery(&record).await?;
            } else if self.is_local_recipient(&did) {
                // The local sender already holds the SENT record of their own
                // group message; projecting it again would double it up in
                // every client and count as unread.
                let own = msg.kind == MsgObjKind::GroupMsg && did == msg.from;
                if !own {
                    let mut projection = Self::build_mailbox_record(
                        did.clone(),
                        MailboxKind::Inbox,
                        msg,
                        RecipientState::Unread,
                        None,
                        vec![
                            format!("group:{}", group.to_string()),
                            format!("session_seq:{session_seq}"),
                        ],
                        "group-projection",
                    )?;
                    projection.session_id = Some(local_key.clone());
                    projection.mailbox =
                        MailboxAddress::new(did.clone(), projection.session_id.clone())
                            .map_err(invalid)?;
                    projection.sort_key = accepted_at_ms;
                    projection.to = group.clone();
                    tx.mailbox(&projection, msg).await?;
                }
                let sql=self.groups.db.render_sql("INSERT INTO owner_sessions(owner,session_id,lifecycle,registered,origin,peer_did,binding_json,created_at_ms,updated_at_ms) VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(owner,session_id) DO NOTHING");
                let binding = json!({"authority_did":group,"session_key":local_key});
                sqlx::query(&sql)
                    .bind(d)
                    .bind(&local_key)
                    .bind("active")
                    .bind(1_i64)
                    .bind("group")
                    .bind(group.to_string())
                    .bind(binding.to_string())
                    .bind(accepted_at_ms as i64)
                    .bind(accepted_at_ms as i64)
                    .execute(&mut *tx.tx)
                    .await
                    .map_err(db_error)?;
            }
        }
        Ok(meta)
    }
    async fn group_event(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        s: Option<&str>,
        action: &str,
        subject: Option<DID>,
        data: Value,
    ) -> Result<()> {
        let seq = Self::allocate(&mut tx.state, s);
        tx.state.refresh_intervals(seq.0);
        let payload = json!({"schema_version":1,"event_id":revision(),"action":action,"actor_did":a.did,"subject_did":subject,"target":if s.is_some(){json!({"kind":"session","session":{"authority_did":tx.state.group_did,"session_key":session_key(&tx.state.group_did,s)?}})}else{json!({"kind":"entity","entity_did":tx.state.group_did})},"occurred_at_ms":seq.2,"changes":data,"source":{"kind":"native","producer_did":tx.state.group_did}});
        let machine_data =
            serde_json::from_value::<BTreeMap<String, CanonValue>>(payload).map_err(db_error)?;
        let msg = MsgObject {
            from: tx.state.group_did.clone(),
            to: vec![tx.state.group_did.clone()],
            to_session: s.map(str::to_owned),
            kind: MsgObjKind::Event,
            created_at_ms: seq.2,
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: action.into(),
                machine: Some(MachineContent {
                    intent: Some("buckyos.action_log".into()),
                    data: machine_data,
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let additional = subject
            .as_ref()
            .map(|d| BTreeSet::from([d.to_string()]))
            .unwrap_or_default();
        self.append_group_message(
            tx,
            &msg,
            None,
            seq,
            if s.is_some() { "session" } else { "member" },
            subject,
            data,
            additional,
        )
        .await?;
        Ok(())
    }

    async fn create_group_session(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        p: &Value,
        guest_request: bool,
    ) -> Result<Value> {
        if !guest_request {
            tx.state.require_cap(a, "session.create", None)?;
        }
        let c = tx.state.config()?;
        if tx
            .state
            .sessions
            .values()
            .filter(|s| s.lifecycle == "active")
            .count()
            >= c.limits.max_sessions
        {
            return Err(denied("session-limit"));
        }
        let sid = optional::<String>(p, "session_id")?.unwrap_or_else(revision);
        validate_sid(&sid)?;
        if tx.state.sessions.contains_key(&sid) {
            return Err(invalid("session-id-already-used"));
        }
        let template: Option<String> = optional(p, "template")?;
        let t = template
            .as_ref()
            .map(|t| {
                c.session_templates
                    .get(t)
                    .cloned()
                    .ok_or_else(|| invalid("unknown-session-template"))
            })
            .transpose()?
            .unwrap_or_default();
        let membership = optional(p, "membership")?.unwrap_or(t.membership);
        let overrides = p.get("rule_overrides").cloned().unwrap_or(json!({}));
        if !overrides.is_object() {
            return Err(invalid("rule-overrides-must-be-object"));
        }
        let mut rules = encoded(t.rules)?;
        merge(&mut rules, &overrides);
        let rules: SessionRules =
            serde_json::from_value(rules).map_err(|e| invalid(e.to_string()))?;
        validate_rules(&membership, &rules, false)?;
        let rev = revision();
        let address = session_key(&tx.state.group_did, Some(&sid))?;
        let mut shared = json!({"schema_version":1,"session":{"authority_did":tx.state.group_did,"session_key":address},"revision":revision(),"updated_at_ms":Self::now_ms(),"extensions":{}});
        for k in ["title", "description", "announcement"] {
            if let Some(v) = p.get(k) {
                let text = v
                    .as_str()
                    .ok_or_else(|| invalid("invalid-session-text"))?
                    .trim();
                let max = if k == "title" { 64 } else { 1024 };
                if text.is_empty() || text.chars().count() > max {
                    return Err(invalid("invalid-session-text"));
                }
                shared[k] = json!(text);
            }
        }
        tx.state.sessions.insert(
            sid.clone(),
            GroupSessionRecord {
                group_did: tx.state.group_did.clone(),
                session_id: sid.clone(),
                template,
                membership: membership.clone(),
                rule_overrides: overrides,
                lifecycle: "active".into(),
                revision: rev.clone(),
                created_by: a.did.clone(),
                created_at_ms: Self::now_ms(),
                shared_state: shared,
                member_states: BTreeMap::new(),
                guest_request: if guest_request {
                    optional(p, "request_id")?
                } else {
                    None
                },
            },
        );
        let mut members = optional::<Vec<DID>>(p, "members")?.unwrap_or_default();
        if membership == SessionMembership::Explicit && !guest_request && !members.contains(&a.did)
        {
            members.push(a.did.clone());
        }
        for d in members {
            Self::include_session_member(&mut tx.state, a, &sid, &d)?;
        }
        tx.state
            .shared_states
            .insert(sid.clone(), tx.state.sessions[&sid].shared_state.clone());
        self.group_event(tx, a, Some(&sid), "session.created", None, json!([]))
            .await?;
        Ok(
            json!({"group_did":tx.state.group_did,"session_id":sid,"session":address,"revision":rev}),
        )
    }
    fn include_session_member(g: &mut GroupState, a: &GroupActor, s: &str, d: &DID) -> Result<()> {
        let m = g
            .members
            .get(&d.to_string())
            .filter(|m| m.state == MemberStatus::Active && !g.blocked(d))
            .ok_or_else(|| denied("session-member-must-be-active-group-member"))?;
        let r = SessionMembershipRecord {
            group_did: g.group_did.clone(),
            session_id: Some(s.into()),
            member_did: d.clone(),
            kind: "group_member".into(),
            epoch: m.epoch,
            state: "included".into(),
            since_seq: g.group_seq + 1,
            actor: a.did.clone(),
        };
        g.participants
            .entry(s.into())
            .or_default()
            .insert(d.to_string(), r);
        Ok(())
    }
    async fn notify_group_member(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        d: &DID,
        action: &str,
        data: Value,
    ) -> Result<()> {
        let msg = MsgObject {
            from: a.did.clone(),
            to: vec![d.clone()],
            kind: MsgObjKind::Operation,
            created_at_ms: Self::now_ms(),
            nonce: Some(tx.state.group_seq),
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: action.into(),
                machine: Some(MachineContent {
                    intent: Some("buckyos.group_invitation".into()),
                    data: serde_json::from_value(
                        json!({"group_did":tx.state.group_did,"action":action,"data":data}),
                    )
                    .map_err(db_error)?,
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        tx.object(&msg, None).await?;
        if self.is_local_recipient(d) {
            let access = self
                .contact_mgr
                .peek_access_permission(&a.did, None, Some(d))
                .await?;
            if access.target_box == "DROP" {
                return Ok(());
            }
            let kind = if access.target_box == "REQUEST_BOX" {
                MailboxKind::RequestBox
            } else {
                MailboxKind::Inbox
            };
            let record = Self::build_mailbox_record(
                d.clone(),
                kind,
                &msg,
                RecipientState::Unread,
                None,
                vec![],
                "group-notification",
            )?;
            tx.mailbox(&record, &msg).await?;
        } else {
            let id = msg.gen_obj_id().0;
            let envelope = self
                .build_delivery_envelope(&id, d.clone())
                .map_err(denied)?;
            let now = Self::now_ms();
            let record = DeliveryRecord {
                delivery_id: Self::build_delivery_id(&id, d, &envelope.transport_did),
                envelope,
                state: DeliveryState::Wait,
                attempts: 0,
                next_retry_at_ms: None,
                external_msg_id: None,
                delivered_at_ms: None,
                last_error: None,
                created_at_ms: now,
                updated_at_ms: now,
            };
            tx.delivery(&record).await?;
        }
        Ok(())
    }
    async fn invite_group_member(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        p: &Value,
    ) -> Result<Value> {
        tx.state.require_cap(a, "group.invite_member", None)?;
        let d: DID = field(p, "member_did")?;
        if tx.state.blocked(&d) {
            return Err(denied("blocked"));
        }
        let role = optional::<GroupRole>(p, "role")?.unwrap_or(GroupRole::Member);
        if role == GroupRole::Owner
            || (role == GroupRole::Admin && !tx.state.capability(&a.did, "group.update_role"))
        {
            return Err(denied("role-not-allowed"));
        }
        let old = tx.state.members.get(&d.to_string());
        if old.is_some_and(|m| {
            matches!(
                m.state,
                MemberStatus::Active | MemberStatus::PendingAdminApproval
            )
        }) {
            return Err(invalid("member-already-participating"));
        }
        let epoch = old.map(|m| m.epoch).unwrap_or(0);
        let entity_kind = match old.map(|m| m.entity_kind.as_str()) {
            Some(kind) if kind != "unknown" => kind.to_string(),
            _ => self.resolve_entity_kind(&d).await?,
        };
        let id = revision();
        let expires =
            optional::<u64>(p, "expires_at_ms")?.unwrap_or(Self::now_ms() + 7 * 86400_000);
        if expires <= Self::now_ms() {
            return Err(invalid("invite-already-expired"));
        }
        tx.state.members.insert(
            d.to_string(),
            MemberRecord {
                member_did: d.clone(),
                role,
                state: MemberStatus::Invited,
                epoch,
                entity_kind,
                invitation_id: Some(id.clone()),
                invited_by: Some(a.did.clone()),
                expires_at_ms: Some(expires),
                since_seq: tx.state.group_seq + 1,
            },
        );
        self.group_event(
            tx,
            a,
            None,
            "entity.member_invited",
            Some(d.clone()),
            json!([]),
        )
        .await?;
        let (consent, recipient) = self.member_consent(&a.did, &d).await?;
        if consent == MemberConsent::Accept {
            let invitee = GroupActor {
                did: d.clone(),
                client: None,
                remote: false,
            };
            self.accept_group_invitation(tx, &invitee, &id, true)
                .await?;
        }
        let state = tx.state.members[&d.to_string()].state;
        if consent != MemberConsent::Drop {
            let mut data =
                json!({"invite_id":id,"role":role,"expires_at_ms":expires,"state":state});
            if recipient != d {
                data["member_did"] = json!(d);
            }
            self.notify_group_member(tx, a, &recipient, "invite", data)
                .await?;
        }
        Ok(json!({"invite_id":id,"member_did":d,"expires_at_ms":expires,"state":state}))
    }
    /// Accept the member's pending invitation. The group side has already
    /// agreed when the inviter can approve members; otherwise the acceptance
    /// waits for admin approval. `automatic` marks acceptances made by the
    /// member's own Zone while delivering the invitation.
    async fn accept_group_invitation(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        invitation_id: &str,
        automatic: bool,
    ) -> Result<Value> {
        if tx.state.blocked(&a.did) {
            return Err(denied("blocked"));
        }
        if !automatic {
            tx.state.check_client(a)?;
        }
        let prior = tx.state.members.get(&a.did.to_string()).cloned();
        if prior.as_ref().is_some_and(|m| {
            matches!(
                m.state,
                MemberStatus::Active | MemberStatus::PendingAdminApproval
            )
        }) {
            return Err(invalid("member-already-participating"));
        }
        let invite = prior
            .as_ref()
            .filter(|m| {
                m.state == MemberStatus::Invited
                    && m.expires_at_ms.is_none_or(|t| t > Self::now_ms())
                    && m.invitation_id.as_deref() == Some(invitation_id)
            })
            .ok_or_else(|| denied("invitation-mismatch"))?;
        let approval = !invite
            .invited_by
            .as_ref()
            .is_some_and(|i| tx.state.capability(i, "group.approve_member"));
        let role = invite.role;
        let invited_by = invite.invited_by.clone();
        if !automatic {
            Self::rate(&mut tx.state, &a.did, "join", false)?;
        }
        self.join_group(tx, a, role, approval, invited_by).await
    }
    /// Join by own request or by invite link. Links count as prior admin
    /// approval unless they require it; plain requests follow `join_policy`.
    async fn request_group_join(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        invite: Option<String>,
    ) -> Result<Value> {
        if tx.state.blocked(&a.did) {
            return Err(denied("blocked"));
        }
        tx.state.check_client(a)?;
        let prior = tx.state.members.get(&a.did.to_string()).cloned();
        if prior.as_ref().is_some_and(|m| {
            matches!(
                m.state,
                MemberStatus::Active | MemberStatus::PendingAdminApproval
            )
        }) {
            return Err(invalid("member-already-participating"));
        }
        let c = tx.state.config()?;
        let approval = if let Some(token) = invite {
            let link = tx
                .state
                .invite_links
                .get(&token)
                .cloned()
                .ok_or_else(|| denied("invalid-invite-link"))?;
            if link.revoked
                || link.expires_at_ms.is_some_and(|t| t <= Self::now_ms())
                || link.max_uses.is_some_and(|n| link.used >= n)
                || !tx.state.capability(&link.created_by, "group.invite_member")
            {
                return Err(denied("invalid-invite-link"));
            }
            tx.state.invite_links.get_mut(&token).unwrap().used += 1;
            link.require_approval
        } else if c.membership.join_policy == "invite_only" {
            return Err(denied("invite-required"));
        } else {
            c.membership.join_policy != "open"
        };
        Self::rate(&mut tx.state, &a.did, "join", false)?;
        self.join_group(tx, a, GroupRole::Member, approval, None)
            .await
    }
    async fn join_group(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        role: GroupRole,
        approval: bool,
        invited_by: Option<DID>,
    ) -> Result<Value> {
        let prior = tx.state.members.get(&a.did.to_string()).cloned();
        let entity_kind = match prior.as_ref().map(|m| m.entity_kind.as_str()) {
            Some(kind) if kind != "unknown" => kind.to_string(),
            _ => self.resolve_entity_kind(&a.did).await?,
        };
        tx.state.members.insert(
            a.did.to_string(),
            MemberRecord {
                member_did: a.did.clone(),
                role,
                state: MemberStatus::PendingAdminApproval,
                epoch: prior.map(|m| m.epoch).unwrap_or(0),
                entity_kind,
                invitation_id: None,
                invited_by: invited_by.clone(),
                expires_at_ms: None,
                since_seq: tx.state.group_seq + 1,
            },
        );
        if approval {
            self.group_event(
                tx,
                a,
                None,
                "entity.member_requested",
                Some(a.did.clone()),
                json!([]),
            )
            .await?;
            let approvers: Vec<_> = tx
                .state
                .members
                .values()
                .filter(|m| tx.state.capability(&m.member_did, "group.approve_member"))
                .map(|m| m.member_did.clone())
                .collect();
            for d in approvers {
                self.notify_group_member(
                    tx,
                    a,
                    &d,
                    "pending_approval",
                    json!({"member_did":a.did,"invited_by":invited_by}),
                )
                .await?;
            }
        } else {
            self.activate_member(tx, a, &a.did).await?;
        }
        encoded(tx.state.members.get(&a.did.to_string()).unwrap())
    }
    fn rate(g: &mut GroupState, d: &DID, s: &str, agent: bool) -> Result<()> {
        let c = g.config()?;
        let now = Self::now_ms();
        let limit = if agent {
            c.limits.agent_messages_per_window
        } else {
            c.limits.messages_per_window
        };
        let items = g
            .rates
            .entry(format!("{}|{}", d.to_string(), s))
            .or_default();
        items.retain(|t| now.saturating_sub(*t) < c.limits.rate_window_ms);
        if items.len() >= limit {
            return Err(kRPC::RPCErrors::ReasonError("rate-limited".into()));
        }
        items.push(now);
        Ok(())
    }
    async fn activate_member(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        d: &DID,
    ) -> Result<()> {
        if tx.state.blocked(d) {
            return Err(denied("blocked"));
        }
        if tx
            .state
            .members
            .values()
            .filter(|m| m.state == MemberStatus::Active)
            .count()
            >= tx.state.config()?.limits.max_members
        {
            return Err(denied("member-limit"));
        }
        if !self
            .run_group_hooks(
                &tx.state,
                &GroupActor {
                    did: d.clone(),
                    ..a.clone()
                },
                None,
                "join",
                true,
                None,
            )
            .await?
        {
            return Err(denied("join-not-allowed"));
        }
        let entity_kind = match tx.state.members.get(&d.to_string()) {
            Some(m) if m.entity_kind != "unknown" => None,
            Some(_) => Some(self.resolve_entity_kind(d).await?),
            None => return Err(missing()),
        };
        let m = tx
            .state
            .members
            .get_mut(&d.to_string())
            .ok_or_else(missing)?;
        if let Some(kind) = entity_kind {
            m.entity_kind = kind;
        }
        m.state = MemberStatus::Active;
        m.epoch += 1;
        m.since_seq = tx.state.group_seq + 1;
        let epoch = m.epoch;
        for participants in tx.state.participants.values_mut() {
            if let Some(r) = participants
                .get_mut(&d.to_string())
                .filter(|r| r.kind == "guest" && r.state == "included")
            {
                r.kind = "group_member".into();
                r.epoch = epoch;
                r.since_seq = tx.state.group_seq + 1;
            }
        }
        self.group_event(
            tx,
            a,
            None,
            "entity.member_joined",
            Some(d.clone()),
            json!([]),
        )
        .await
    }
    async fn mutate_group(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        method: &str,
        p: &Value,
        attestation: Option<Value>,
    ) -> Result<Value> {
        let s: Option<String> = optional(p, "session_id")?;
        if let Some(att) = attestation {
            tx.state.audit.push(json!({"actor":att["attested_by"],"member":att["member_did"],"action":"tunnel-attestation","source_event":att["source_event"],"at_ms":Self::now_ms()}));
        }
        match method {
            "group.apply_config" => {
                tx.state.require_cap(a, "group.update_config", None)?;
                let old = tx.state.config()?;
                expected(p, &old.revision)?;
                let patch: Value = field(p, "patch")?;
                if !patch.is_object() {
                    return Err(invalid("patch-must-be-object"));
                }
                let before = tx.state.configuration.clone();
                merge(&mut tx.state.configuration, &patch);
                if before == tx.state.configuration {
                    return Ok(json!({"revision":old.revision}));
                }
                let mut c = tx.state.config()?;
                c.revision = revision();
                c.validate()?;
                for r in tx
                    .state
                    .sessions
                    .values()
                    .filter(|r| r.lifecycle != "deleted")
                {
                    let rules = tx.state.rules(Some(&r.session_id))?;
                    validate_rules(&r.membership, &rules, false)?;
                }
                if tx
                    .state
                    .members
                    .values()
                    .filter(|m| m.state == MemberStatus::Active)
                    .count()
                    > c.limits.max_members
                    || tx
                        .state
                        .sessions
                        .values()
                        .filter(|s| s.lifecycle == "active")
                        .count()
                        > c.limits.max_sessions
                {
                    return Err(invalid("limits-below-current-usage"));
                }
                tx.state.configuration["revision"] = json!(c.revision);
                tx.state.doc_updated_at_ms = tx.state.doc_updated_at_ms.max(Self::now_ms());
                tx.state
                    .config_history
                    .insert(c.revision.clone(), tx.state.configuration.clone());
                let changes: Vec<_> = tx.state.configuration.as_object().unwrap().iter().filter(|(k, v)| before.get(*k) != Some(*v)).map(|(k, v)| json!({"field":k,"before":before.get(k).map(|v|json!({"present":true,"value":v})).unwrap_or(json!({"present":false})),"after":{"present":true,"value":v}})).collect();
                self.group_event(tx, a, None, "entity.config_changed", None, json!(changes))
                    .await?;
                Ok(json!({"revision":c.revision}))
            }
            "group.archive" | "group.delete" => {
                if tx.state.owner != a.did {
                    return Err(denied("owner-required"));
                }
                tx.state.check_client(a)?;
                tx.state.doc_updated_at_ms = tx.state.doc_updated_at_ms.max(Self::now_ms());
                let action = if method == "group.delete" {
                    "entity.group_deleted"
                } else {
                    "entity.group_archived"
                };
                self.group_event(tx, a, None, action, None, json!([]))
                    .await?;
                if method == "group.delete" {
                    let guests: BTreeSet<_> = tx
                        .state
                        .participants
                        .values()
                        .flat_map(|p| p.values())
                        .filter(|m| m.kind == "guest" && m.state == "included")
                        .map(|m| m.member_did.to_string())
                        .collect();
                    if let Some(change) = tx.state.changes.last_mut() {
                        change.audience.extend(guests);
                    }
                    let sessions: Vec<_> = tx.state.sessions.keys().cloned().collect();
                    for sid in sessions {
                        tx.delete_session_objects(Some(&sid)).await?;
                    }
                    tx.delete_session_objects(None).await?;
                    tx.state.lifecycle = "deleted".into();
                    let readers: Vec<String> = tx
                        .state
                        .members
                        .keys()
                        .cloned()
                        .chain(
                            tx.state
                                .participants
                                .values()
                                .flat_map(|p| p.keys().cloned()),
                        )
                        .collect();
                    tx.state.tombstone_readers.extend(readers);
                    tx.state
                        .changes
                        .retain(|c| c.data["action"] == "entity.group_deleted");
                    tx.state.members.clear();
                    tx.state.participants.clear();
                    tx.state.pending_owner_transfer = None;
                    tx.state.invite_links.clear();
                    tx.state.configuration = json!({});
                    tx.state.config_history.clear();
                    tx.state.sessions.clear();
                    tx.state.intervals.clear();
                    tx.state.moderation.clear();
                    tx.state.shared_states.clear();
                    tx.state.member_states.clear();
                    tx.state.rates.clear();
                    tx.state.read_markers.clear();
                    tx.state.operations.clear();
                    tx.state.audit.clear();
                } else {
                    tx.state.lifecycle = "archived".into();
                }
                Ok(json!({"lifecycle":tx.state.lifecycle}))
            }
            "group.transfer_owner" => {
                if tx.state.controller != a.did {
                    return Err(denied("controller-required"));
                }
                tx.state.check_client(a)?;
                let d: DID = field(p, "member_did")?;
                if d == tx.state.owner {
                    return Err(invalid("already-owner"));
                }
                if tx.state.role(&d).is_none() || tx.state.blocked(&d) {
                    return Err(missing());
                }
                let transfer = OwnerTransfer {
                    member_did: d.clone(),
                    transfer_id: revision(),
                    expires_at_ms: optional::<u64>(p, "expires_at_ms")?
                        .unwrap_or(Self::now_ms() + 7 * 86400_000),
                };
                if transfer.expires_at_ms <= Self::now_ms() {
                    return Err(invalid("transfer-already-expired"));
                }
                tx.state.pending_owner_transfer = Some(transfer.clone());
                self.group_event(
                    tx,
                    a,
                    None,
                    "entity.owner_transfer_offered",
                    Some(d.clone()),
                    json!([]),
                )
                .await?;
                self.notify_group_member(
                    tx,
                    a,
                    &d,
                    "owner_transfer",
                    json!({"transfer_id":transfer.transfer_id,"expires_at_ms":transfer.expires_at_ms}),
                )
                .await?;
                Ok(
                    json!({"transfer_id":transfer.transfer_id,"member_did":d,"expires_at_ms":transfer.expires_at_ms}),
                )
            }
            "group.accept_owner_transfer" => {
                let id: String = field(p, "transfer_id")?;
                let transfer = tx
                    .state
                    .pending_owner_transfer
                    .clone()
                    .filter(|t| {
                        t.transfer_id == id
                            && t.member_did == a.did
                            && t.expires_at_ms > Self::now_ms()
                    })
                    .ok_or_else(|| denied("transfer-mismatch"))?;
                if tx.state.role(&a.did).is_none() || tx.state.blocked(&a.did) {
                    return Err(missing());
                }
                tx.state.check_client(a)?;
                let previous = tx.state.owner.clone();
                if let Some(m) = tx.state.members.get_mut(&previous.to_string()) {
                    m.role = GroupRole::Admin;
                }
                tx.state
                    .members
                    .get_mut(&transfer.member_did.to_string())
                    .unwrap()
                    .role = GroupRole::Owner;
                tx.state.owner = transfer.member_did.clone();
                tx.state.pending_owner_transfer = None;
                tx.state.doc_updated_at_ms = tx.state.doc_updated_at_ms.max(Self::now_ms());
                self.group_event(
                    tx,
                    a,
                    None,
                    "entity.owner_changed",
                    Some(transfer.member_did),
                    json!([]),
                )
                .await?;
                Ok(json!({"owner":tx.state.owner}))
            }
            "group.cancel_owner_transfer" => {
                if tx.state.controller != a.did {
                    return Err(denied("controller-required"));
                }
                let transfer = tx.state.pending_owner_transfer.take().ok_or_else(missing)?;
                self.group_event(
                    tx,
                    a,
                    None,
                    "entity.owner_transfer_cancelled",
                    Some(transfer.member_did),
                    json!([]),
                )
                .await?;
                Ok(json!({"cancelled":true}))
            }
            "group.invite_member" => self.invite_group_member(tx, a, p).await,
            "group.revoke_invite" => {
                tx.state.require_cap(a, "group.invite_member", None)?;
                let d: DID = field(p, "member_did")?;
                let m = tx
                    .state
                    .members
                    .get_mut(&d.to_string())
                    .filter(|m| m.state == MemberStatus::Invited)
                    .ok_or_else(missing)?;
                m.state = MemberStatus::Revoked;
                self.group_event(tx, a, None, "entity.invite_revoked", Some(d), json!([]))
                    .await?;
                Ok(json!({"state":"revoked"}))
            }
            "group.create_invite_link" => {
                tx.state.require_cap(a, "group.invite_member", None)?;
                let link = GroupInviteLink {
                    token: format!(
                        "{}{}",
                        uuid::Uuid::new_v4().simple(),
                        uuid::Uuid::new_v4().simple()
                    ),
                    created_by: a.did.clone(),
                    expires_at_ms: optional(p, "expires_at_ms")?,
                    max_uses: optional(p, "max_uses")?,
                    used: 0,
                    require_approval: optional(p, "require_approval")?.unwrap_or(false),
                    revoked: false,
                };
                if link.expires_at_ms.is_some_and(|t| t <= Self::now_ms())
                    || link.max_uses == Some(0)
                {
                    return Err(invalid("invalid-invite-link"));
                }
                tx.state
                    .invite_links
                    .insert(link.token.clone(), link.clone());
                self.group_event(tx, a, None, "entity.invite_link_created", None, json!([]))
                    .await?;
                encoded(link)
            }
            "group.revoke_invite_link" => {
                tx.state.require_cap(a, "group.invite_member", None)?;
                let token: String = field(p, "token")?;
                tx.state
                    .invite_links
                    .get_mut(&token)
                    .ok_or_else(missing)?
                    .revoked = true;
                self.group_event(tx, a, None, "entity.invite_link_revoked", None, json!([]))
                    .await?;
                Ok(json!({"revoked":true}))
            }
            "group.accept_invitation" => {
                let id: String = field(p, "invitation_id")?;
                self.accept_group_invitation(tx, a, &id, false).await
            }
            "group.request_join" => self.request_group_join(tx, a, optional(p, "invite")?).await,
            "group.approve_member" | "group.reject_member" => {
                tx.state.require_cap(a, "group.approve_member", None)?;
                let d: DID = field(p, "member_did")?;
                if tx.state.blocked(&d)
                    || !tx
                        .state
                        .members
                        .get(&d.to_string())
                        .is_some_and(|m| m.state == MemberStatus::PendingAdminApproval)
                {
                    return Err(denied("member-not-pending"));
                }
                if method == "group.approve_member" {
                    self.activate_member(tx, a, &d).await?;
                } else {
                    tx.state.members.get_mut(&d.to_string()).unwrap().state =
                        MemberStatus::Rejected;
                    self.group_event(
                        tx,
                        a,
                        None,
                        "entity.member_rejected",
                        Some(d.clone()),
                        json!([]),
                    )
                    .await?;
                    self.notify_group_member(tx, a, &d, "rejected", json!({}))
                        .await?;
                }
                encoded(tx.state.members.get(&d.to_string()).unwrap())
            }
            "group.leave" | "group.remove_member" => {
                let d = if method == "group.leave" {
                    a.did.clone()
                } else {
                    tx.state.require_cap(a, "group.remove_member", None)?;
                    field::<DID>(p, "member_did")?
                };
                if d == tx.state.owner {
                    return Err(denied("owner-must-transfer-first"));
                }
                if tx.state.role(&d).is_none() {
                    return Err(missing());
                }
                tx.state.check_client(a)?;
                tx.state.members.get_mut(&d.to_string()).unwrap().state = if method == "group.leave"
                {
                    MemberStatus::Left
                } else {
                    MemberStatus::Removed
                };
                self.group_event(
                    tx,
                    a,
                    None,
                    if method == "group.leave" {
                        "entity.member_left"
                    } else {
                        "entity.member_removed"
                    },
                    Some(d.clone()),
                    json!([]),
                )
                .await?;
                if method == "group.remove_member" {
                    self.notify_group_member(tx, a, &d, "removed", json!({}))
                        .await?;
                }
                Ok(json!({"ok":true}))
            }
            "group.update_member_role" => {
                tx.state.require_cap(a, "group.update_role", None)?;
                let d: DID = field(p, "member_did")?;
                let role: GroupRole = field(p, "role")?;
                if role == GroupRole::Owner || d == tx.state.owner {
                    return Err(denied("use-transfer-owner"));
                }
                if tx.state.role(&d).is_none() || tx.state.blocked(&d) {
                    return Err(missing());
                }
                tx.state.members.get_mut(&d.to_string()).unwrap().role = role;
                self.group_event(
                    tx,
                    a,
                    None,
                    "entity.member_role_changed",
                    Some(d),
                    json!([]),
                )
                .await?;
                Ok(json!({"role":role}))
            }
            "group.moderate" => {
                tx.state.require_cap(a, "group.moderate", None)?;
                let d: DID = field(p, "member_did")?;
                if d == tx.state.owner {
                    return Err(denied("cannot-moderate-owner"));
                }
                let block: Option<bool> = optional(p, "blocked")?;
                let mute: Option<u64> = optional(p, "muted_until_ms")?;
                let m = tx.state.moderation.entry(d.to_string()).or_default();
                if let Some(b) = block {
                    m.blocked = b;
                }
                if p.get("muted_until_ms").is_some() {
                    m.muted_until_ms = mute;
                }
                if block == Some(true) {
                    let member_removed = tx.state.role(&d).is_some();
                    let guest_sessions: Vec<_> = tx
                        .state
                        .participants
                        .iter()
                        .filter(|(_, p)| {
                            p.get(&d.to_string())
                                .is_some_and(|r| r.kind == "guest" && r.state == "included")
                        })
                        .map(|(sid, _)| sid.clone())
                        .collect();
                    if let Some(m) = tx.state.members.get_mut(&d.to_string()) {
                        m.state = MemberStatus::Removed;
                    }
                    for participants in tx.state.participants.values_mut() {
                        if let Some(r) = participants.get_mut(&d.to_string()) {
                            r.state = "removed".into();
                        }
                    }
                    if member_removed {
                        self.group_event(
                            tx,
                            a,
                            None,
                            "entity.member_removed",
                            Some(d.clone()),
                            json!([]),
                        )
                        .await?;
                        self.notify_group_member(tx, a, &d, "removed", json!({"blocked":true}))
                            .await?;
                    }
                    for sid in guest_sessions {
                        self.group_event(
                            tx,
                            a,
                            Some(&sid),
                            "session.member_removed",
                            Some(d.clone()),
                            json!([]),
                        )
                        .await?;
                        self.notify_group_member(
                            tx,
                            a,
                            &d,
                            "session_removed",
                            json!({"session_id":sid,"blocked":true}),
                        )
                        .await?;
                    }
                }
                self.group_event(
                    tx,
                    a,
                    None,
                    "entity.moderation_changed",
                    Some(d.clone()),
                    json!([]),
                )
                .await?;
                encoded(tx.state.moderation.get(&d.to_string()).unwrap())
            }
            "group.create_session" => self.create_group_session(tx, a, p, false).await,
            "group.update_session" => {
                let sid = s
                    .as_deref()
                    .ok_or_else(|| invalid("named-session-required"))?;
                tx.state.require_cap(a, "session.manage", Some(sid))?;
                let r = tx
                    .state
                    .sessions
                    .get(sid)
                    .filter(|r| r.lifecycle != "deleted")
                    .ok_or_else(missing)?;
                expected(p, &r.revision)?;
                let r = tx.state.sessions.get_mut(sid).unwrap();
                if let Some(m) = optional(p, "membership")? {
                    r.membership = m;
                }
                if let Some(t) = p.get("template") {
                    r.template = if t.is_null() {
                        None
                    } else {
                        Some(serde_json::from_value(t.clone()).map_err(db_error)?)
                    };
                }
                if let Some(patch) = p.get("rule_overrides") {
                    if !patch.is_object() {
                        return Err(invalid("invalid-rule-overrides"));
                    }
                    merge(&mut r.rule_overrides, patch);
                }
                r.revision = revision();
                let rev = r.revision.clone();
                let rules = tx.state.rules(Some(sid))?;
                validate_rules(&tx.state.sessions[sid].membership, &rules, false)?;
                for d in optional::<Vec<DID>>(p, "add_members")?.unwrap_or_default() {
                    Self::include_session_member(&mut tx.state, a, sid, &d)?;
                }
                self.group_event(tx, a, Some(sid), "session.rules_changed", None, json!([]))
                    .await?;
                Ok(json!({"revision":rev}))
            }
            "group.archive_session" | "group.delete_session" => {
                let sid = s
                    .as_deref()
                    .ok_or_else(|| invalid("default-session-cannot-be-deleted"))?;
                tx.state.require_cap(a, "session.manage", Some(sid))?;
                let r = tx
                    .state
                    .sessions
                    .get(sid)
                    .filter(|r| r.lifecycle != "deleted")
                    .ok_or_else(missing)?;
                expected(p, &r.revision)?;
                self.group_event(
                    tx,
                    a,
                    Some(sid),
                    if method == "group.delete_session" {
                        "session.deleted"
                    } else {
                        "session.archived"
                    },
                    None,
                    json!([]),
                )
                .await?;
                let deleted = method == "group.delete_session";
                let r = tx.state.sessions.get_mut(sid).unwrap();
                r.lifecycle = if deleted { "deleted" } else { "archived" }.into();
                r.revision = revision();
                let rev = r.revision.clone();
                if deleted {
                    tx.delete_session_objects(Some(sid)).await?;
                    if let Some(participants) = tx.state.participants.get(sid) {
                        let readers: Vec<String> = participants.keys().cloned().collect();
                        tx.state.tombstone_readers.extend(readers);
                    }
                    tx.state.participants.remove(sid);
                    tx.state.shared_states.remove(sid);
                    tx.state.member_states.remove(sid);
                    tx.state.read_markers.remove(sid);
                    let record = tx.state.sessions.get_mut(sid).unwrap();
                    record.template = None;
                    record.rule_overrides = json!({});
                    record.shared_state = json!({});
                    record.member_states.clear();
                }
                tx.state.refresh_intervals(tx.state.group_seq);
                Ok(json!({"revision":rev,"lifecycle":if deleted{"deleted"}else{"archived"}}))
            }
            "group.invite_session_guest" => {
                let sid = s
                    .as_deref()
                    .ok_or_else(|| invalid("named-session-required"))?;
                tx.state.require_cap(a, "session.invite_guest", Some(sid))?;
                if !tx.state.rules(Some(sid))?.allow_guests {
                    return Err(denied("guests-not-allowed"));
                }
                let d: DID = field(p, "member_did")?;
                if tx.state.blocked(&d) || tx.state.role(&d).is_some() {
                    return Err(denied("not-a-session-guest"));
                }
                Self::guest_limit(&tx.state, sid)?;
                let epoch = tx
                    .state
                    .participants
                    .get(sid)
                    .and_then(|m| m.get(&d.to_string()))
                    .map(|r| r.epoch)
                    .unwrap_or(0);
                let r = SessionMembershipRecord {
                    group_did: tx.state.group_did.clone(),
                    session_id: Some(sid.into()),
                    member_did: d.clone(),
                    kind: "guest".into(),
                    epoch,
                    state: "invited".into(),
                    since_seq: tx.state.group_seq + 1,
                    actor: a.did.clone(),
                };
                tx.state
                    .participants
                    .entry(sid.into())
                    .or_default()
                    .insert(d.to_string(), r.clone());
                self.group_event(tx, a, Some(sid), "session.guest_invited", None, json!([]))
                    .await?;
                self.notify_group_member(tx,a,&d,"session_invite",json!({"session_id":sid,"profile":tx.state.config()?.profile,"title":tx.state.sessions[sid].shared_state.get("title")})).await?;
                encoded(r)
            }
            "group.accept_session_invitation" => {
                let sid = s.as_deref().ok_or_else(missing)?;
                if tx.state.blocked(&a.did)
                    || !tx
                        .state
                        .participants
                        .get(sid)
                        .and_then(|p| p.get(&a.did.to_string()))
                        .is_some_and(|r| r.kind == "guest" && r.state == "invited")
                {
                    return Err(missing());
                }
                tx.state.check_client(a)?;
                Self::guest_limit(&tx.state, sid)?;
                Self::rate(&mut tx.state, &a.did, "join", false)?;
                if !self
                    .run_group_hooks(&tx.state, a, Some(sid), "join", true, None)
                    .await?
                {
                    return Err(denied("join-not-allowed"));
                }
                let r = tx
                    .state
                    .participants
                    .get_mut(sid)
                    .unwrap()
                    .get_mut(&a.did.to_string())
                    .unwrap();
                r.state = "included".into();
                r.epoch += 1;
                r.since_seq = tx.state.group_seq + 1;
                self.group_event(
                    tx,
                    a,
                    Some(sid),
                    "session.member_added",
                    Some(a.did.clone()),
                    json!([{"field":"participant_kind","after":{"present":true,"value":"guest"}}]),
                )
                .await?;
                Ok(json!({"session":session_key(&tx.state.group_did,Some(sid))?}))
            }
            "group.submit_guest_request" => {
                if tx.state.blocked(&a.did) || tx.state.role(&a.did).is_some() {
                    return Err(denied("not-a-session-guest"));
                }
                tx.state.check_client(a)?;
                let request_id: String = field(p, "request_id")?;
                validate_sid(&request_id)?;
                if let Some(r) = tx.state.sessions.values().find(|r| {
                    r.guest_request.as_deref() == Some(&request_id) && r.created_by == a.did
                }) {
                    return Ok(
                        json!({"group_did":tx.state.group_did,"session_id":r.session_id,"session":session_key(&tx.state.group_did,Some(&r.session_id))?,"revision":r.revision}),
                    );
                }
                let entry = tx
                    .state
                    .config()?
                    .membership
                    .guest_entry
                    .ok_or_else(|| denied("guest-entry-disabled"))?;
                let open = tx
                    .state
                    .sessions
                    .values()
                    .filter(|r| {
                        r.lifecycle == "active"
                            && r.guest_request.is_some()
                            && r.created_by == a.did
                    })
                    .count();
                if open >= entry.max_open_per_guest {
                    return Err(denied("guest-request-limit"));
                }
                Self::rate(&mut tx.state, &a.did, "join", false)?;
                let result = self
                    .create_group_session(
                        tx,
                        a,
                        &json!({"template":entry.session_template,"request_id":request_id}),
                        true,
                    )
                    .await?;
                let sid = result["session_id"].as_str().unwrap();
                if !self
                    .run_group_hooks(&tx.state, a, Some(sid), "join", true, None)
                    .await?
                {
                    return Err(denied("join-not-allowed"));
                }
                tx.state.participants.entry(sid.into()).or_default().insert(
                    a.did.to_string(),
                    SessionMembershipRecord {
                        group_did: tx.state.group_did.clone(),
                        session_id: Some(sid.into()),
                        member_did: a.did.clone(),
                        kind: "guest".into(),
                        epoch: 1,
                        state: "included".into(),
                        since_seq: tx.state.group_seq + 1,
                        actor: a.did.clone(),
                    },
                );
                self.group_event(
                    tx,
                    a,
                    Some(sid),
                    "session.member_added",
                    Some(a.did.clone()),
                    json!([]),
                )
                .await?;
                Ok(result)
            }
            "group.remove_session_member" | "group.leave_session" => {
                let sid = s
                    .as_deref()
                    .ok_or_else(|| invalid("use-group-leave-for-default-session"))?;
                let d = if method == "group.leave_session" {
                    tx.state.require_session(a, Some(sid), false)?;
                    a.did.clone()
                } else {
                    let d: DID = field(p, "member_did")?;
                    let guest = tx.state.role(&d).is_none();
                    tx.state.require_cap(
                        a,
                        if guest {
                            "session.invite_guest"
                        } else {
                            "session.manage"
                        },
                        Some(sid),
                    )?;
                    let r = tx.state.sessions.get(sid).ok_or_else(missing)?;
                    expected(p, &r.revision)?;
                    d
                };
                if !tx.state.effective(Some(sid), &d) {
                    return Err(missing());
                }
                let epoch = tx
                    .state
                    .members
                    .get(&d.to_string())
                    .map(|m| m.epoch)
                    .unwrap_or_else(|| tx.state.participants[sid][&d.to_string()].epoch);
                let r = SessionMembershipRecord {
                    group_did: tx.state.group_did.clone(),
                    session_id: Some(sid.into()),
                    member_did: d.clone(),
                    kind: if tx.state.role(&d).is_some() {
                        "group_member"
                    } else {
                        "guest"
                    }
                    .into(),
                    epoch,
                    state: if method == "group.leave_session" {
                        "left"
                    } else {
                        "removed"
                    }
                    .into(),
                    since_seq: tx.state.group_seq + 1,
                    actor: a.did.clone(),
                };
                tx.state
                    .participants
                    .entry(sid.into())
                    .or_default()
                    .insert(d.to_string(), r);
                tx.state.sessions.get_mut(sid).unwrap().revision = revision();
                self.group_event(
                    tx,
                    a,
                    Some(sid),
                    if method == "group.leave_session" {
                        "session.member_left"
                    } else {
                        "session.member_removed"
                    },
                    Some(d.clone()),
                    json!([]),
                )
                .await?;
                if method == "group.remove_session_member" {
                    self.notify_group_member(
                        tx,
                        a,
                        &d,
                        "session_removed",
                        json!({"session_id":sid}),
                    )
                    .await?;
                }
                Ok(json!({"revision":tx.state.sessions[sid].revision}))
            }
            "group.update_read_marker" => {
                tx.state.require_session(a, s.as_deref(), false)?;
                let marker: u64 = field(p, "last_read_seq")?;
                let max = tx
                    .state
                    .session_seqs
                    .get(s.as_deref().unwrap_or(""))
                    .copied()
                    .unwrap_or(0);
                if marker > max {
                    return Err(invalid("read-marker-beyond-session"));
                }
                self.run_group_hooks(&tx.state, a, s.as_deref(), "read", true, None)
                    .await?;
                let current = tx
                    .state
                    .read_markers
                    .entry(s.unwrap_or_default())
                    .or_default()
                    .entry(a.did.to_string())
                    .or_default();
                *current = (*current).max(marker);
                Ok(json!({"last_read_seq":current}))
            }
            "group.get_read_markers" => {
                tx.state.require_session(a, s.as_deref(), true)?;
                self.run_group_hooks(&tx.state, a, s.as_deref(), "read", true, None)
                    .await?;
                let c = tx.state.config()?;
                let rules = tx.state.rules(s.as_deref())?;
                let audience = tx.state.audience(s.as_deref());
                let hidden =
                    rules.receipts == "hidden" || audience.len() > c.limits.receipts_max_members;
                let own = tx
                    .state
                    .read_markers
                    .get(s.as_deref().unwrap_or(""))
                    .and_then(|m| m.get(&a.did.to_string()))
                    .copied()
                    .unwrap_or(0);
                if hidden {
                    return Ok(json!({"last_read_seq":own,"visibility":"hidden"}));
                }
                let seq: u64 = field(p, "session_seq")?;
                let meta = tx
                    .state
                    .messages
                    .values()
                    .find(|m| m.session_id == s && m.session_seq == seq)
                    .ok_or_else(missing)?;
                let readers: Vec<_> = audience
                    .into_iter()
                    .filter(|d| {
                        tx.state
                            .read_markers
                            .get(s.as_deref().unwrap_or(""))
                            .and_then(|m| m.get(d))
                            .is_some_and(|n| *n >= seq)
                            && DID::from_str(d).is_ok_and(|d| tx.state.visible_at(&d, meta))
                    })
                    .collect();
                Ok(if rules.receipts == "readers" {
                    json!({"last_read_seq":own,"count":readers.len(),"readers":readers})
                } else {
                    json!({"last_read_seq":own,"count":readers.len()})
                })
            }
            "group.get_shared_state"
            | "group.get_member_state"
            | "group.update_shared_state"
            | "group.update_member_state" => {
                self.group_session_state(tx, a, method, p, s.as_deref())
                    .await
            }
            _ => Err(kRPC::RPCErrors::UnknownMethod(method.into())),
        }
    }
    fn guest_limit(g: &GroupState, s: &str) -> Result<()> {
        if g.participants
            .get(s)
            .map(|p| {
                p.values()
                    .filter(|r| r.kind == "guest" && r.state == "included")
                    .count()
            })
            .unwrap_or(0)
            >= g.config()?.limits.max_guests_per_session
        {
            return Err(denied("guest-limit"));
        }
        Ok(())
    }

    async fn group_session_state(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        method: &str,
        p: &Value,
        s: Option<&str>,
    ) -> Result<Value> {
        tx.state.require_session(a, s, true)?;
        if !self
            .run_group_hooks(&tx.state, a, s, "read", true, None)
            .await?
        {
            return Err(denied("read-not-allowed"));
        }
        let member = method.ends_with("member_state");
        let d = optional::<DID>(p, "member_did")?.unwrap_or(a.did.clone());
        if member && !tx.state.effective(s, &d) {
            return Err(missing());
        }
        let updating = method.starts_with("group.update_");
        if updating
            && ((!member && !tx.state.capability(&a.did, "session.update_shared_state"))
                || (member && d != a.did))
        {
            return Err(denied("state-write-denied"));
        }
        let key = s.unwrap_or("").to_owned();
        let initial = json!({"schema_version":1,"session":{"authority_did":tx.state.group_did,"session_key":session_key(&tx.state.group_did,s)?},"revision":revision(),"updated_at_ms":Self::now_ms(),"extensions":{}});
        let current = if member {
            tx.state
                .member_states
                .entry(key.clone())
                .or_default()
                .entry(d.to_string())
                .or_insert_with(|| {
                    let mut v = initial.clone();
                    v["member_did"] = json!(d);
                    v
                })
                .clone()
        } else {
            tx.state
                .shared_states
                .entry(key.clone())
                .or_insert(initial)
                .clone()
        };
        if !updating {
            return Ok(current);
        }
        expected(p, current["revision"].as_str().unwrap())?;
        let set: BTreeMap<String, Value> = optional(p, "set")?.unwrap_or_default();
        let unset: Vec<String> = optional(p, "unset")?.unwrap_or_default();
        let mut next = current.clone();
        for (k, v) in &set {
            if unset.contains(k) {
                return Err(invalid("field-set-and-unset"));
            }
            let max = match (member, k.as_str()) {
                (true, "nickname") | (false, "title") => 64,
                (false, "description") | (false, "announcement") => 1024,
                _ => return Err(invalid("unknown-state-field")),
            };
            let text = v
                .as_str()
                .ok_or_else(|| invalid("invalid-state-text"))?
                .trim();
            if text.is_empty() || text.chars().count() > max {
                return Err(invalid("invalid-state-text"));
            }
            next[k] = json!(text);
        }
        for k in &unset {
            if !(if member {
                k == "nickname"
            } else {
                ["title", "description", "announcement"].contains(&k.as_str())
            }) {
                return Err(invalid("unknown-state-field"));
            }
            next.as_object_mut().unwrap().remove(k);
        }
        if next == current {
            return Ok(current);
        }
        next["revision"] = json!(revision());
        next["updated_at_ms"] = json!(Self::now_ms());
        let changes:Vec<_>=set.keys().chain(unset.iter()).map(|k|json!({"field":k,"before":current.get(k).map(|v|json!({"present":true,"value":v})).unwrap_or(json!({"present":false})),"after":next.get(k).map(|v|json!({"present":true,"value":v})).unwrap_or(json!({"present":false}))})).collect();
        if member {
            tx.state
                .member_states
                .entry(key)
                .or_default()
                .insert(d.to_string(), next.clone());
        } else {
            tx.state.shared_states.insert(key, next.clone());
        }
        self.group_event(
            tx,
            a,
            s,
            if member {
                "session.member_state_changed"
            } else if set.len() == 1 && set.contains_key("title") && unset.is_empty() {
                "session.title_changed"
            } else {
                "session.shared_state_changed"
            },
            if member { Some(d) } else { None },
            json!(changes),
        )
        .await?;
        Ok(next)
    }

    pub(crate) async fn group_sessions(&self, a: &GroupActor, group: &DID) -> Result<Value> {
        let g = self
            .groups
            .load(group)
            .await?
            .filter(|g| g.lifecycle != "deleted")
            .ok_or_else(missing)?;
        g.check_client(a)?;
        if g.role(&a.did).is_none() && !g.sessions.keys().any(|s| g.effective(Some(s), &a.did)) {
            return Err(missing());
        }
        let mut items = vec![];
        for sid in std::iter::once(None).chain(g.sessions.keys().map(|s| Some(s.as_str()))) {
            if g.require_session(a, sid, true).is_err() {
                continue;
            }
            if !self
                .run_group_hooks(&g, a, sid, "read", true, None)
                .await
                .unwrap_or(false)
            {
                continue;
            }
            let rule = g.rules(sid)?;
            let audience = g.audience(sid);
            let has_guests = g.participants.get(sid.unwrap_or("")).is_some_and(|p| {
                p.values().any(|r| {
                    r.kind == "guest" && r.state == "included" && g.effective(sid, &r.member_did)
                })
            });
            let shared = g
                .shared_states
                .get(sid.unwrap_or(""))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut item = json!({"session_id":sid,"session":session_key(group,sid)?,"state_ref":{"authority_did":group,"session_key":session_key(group,sid)?},"shared_state":shared,"has_guests":has_guests,"owner_can_read":true,"rules":rule,"lifecycle":sid.map(|s|g.sessions[s].lifecycle.as_str()).unwrap_or(&g.lifecycle),"revision":sid.map(|s|g.sessions[s].revision.clone()).unwrap_or(g.config()?.revision)});
            if g.role(&a.did).is_some() {
                item["participant_count"] = json!(audience.len());
            }
            items.push(item);
        }
        let doc = if g.role(&a.did).is_some() {
            Some(self.group_doc(&g)?)
        } else {
            None
        };
        Ok(json!({"items":items,"group_doc":doc}))
    }
    /// Participants of one Session as the reader may see them (v2 §3.3): a
    /// reader who may see the group's member list gets every effective
    /// participant (`complete`); a Session Guest, or a member while the list is
    /// hidden from them, gets only the explicit participants and those who
    /// have posted in the Session, never the members a `Roles` / `Inherit`
    /// rule brings in. Pending guest invitations are listed for whoever may
    /// invite guests.
    pub(crate) async fn group_session_members(
        &self,
        a: &GroupActor,
        group: &DID,
        s: Option<&str>,
    ) -> Result<Value> {
        let g = self
            .groups
            .load(group)
            .await?
            .filter(|g| g.lifecycle != "deleted")
            .ok_or_else(missing)?;
        g.require_session(a, s, true)?;
        if !self.run_group_hooks(&g, a, s, "read", true, None).await? {
            return Err(denied("read-not-allowed"));
        }
        let records = g.participants.get(s.unwrap_or(""));
        let complete = g.capability(&a.did, "group.read_all")
            || (g.role(&a.did).is_some()
                && (g.config()?.membership.member_list_visibility != "admins_only"
                    || g.capability(&a.did, "group.approve_member")));
        let shown = if complete {
            g.audience(s)
        } else {
            let mut known: BTreeSet<String> = records
                .into_iter()
                .flatten()
                .filter(|(_, r)| r.state == "included")
                .map(|(d, _)| d.clone())
                .collect();
            known.extend(
                g.messages
                    .values()
                    .filter(|m| m.session_id.as_deref() == s && !m.redacted)
                    .map(|m| m.from.to_string()),
            );
            known.insert(a.did.to_string());
            known
                .into_iter()
                .filter(|d| DID::from_str(d).is_ok_and(|d| g.effective(s, &d)))
                .collect()
        };
        let mut items = vec![];
        for d in shown {
            let member = g
                .members
                .get(&d)
                .filter(|m| m.state == MemberStatus::Active);
            items.push(json!({"member_did":d,"kind":if member.is_some(){"group_member"}else{"guest"},"role":member.map(|m|m.role),"entity_kind":member.map(|m|&m.entity_kind),"state":"included"}));
        }
        if s.is_some() && g.require_cap(a, "session.invite_guest", s).is_ok() {
            for (d, r) in records.into_iter().flatten() {
                if r.kind == "guest" && r.state == "invited" && !g.blocked(&r.member_did) {
                    items.push(json!({"member_did":d,"kind":"guest","role":null,"entity_kind":null,"state":"invited"}));
                }
            }
        }
        let revision = match s {
            Some(s) => g.sessions[s].revision.clone(),
            None => g.config()?.revision,
        };
        Ok(json!({"items":items,"complete":complete,"revision":revision}))
    }
    pub(crate) async fn group_inbox(
        &self,
        a: &GroupActor,
        group: &DID,
        s: Option<&str>,
        after: u64,
        limit: usize,
    ) -> Result<Value> {
        let g = self.groups.load(group).await?.ok_or_else(missing)?;
        g.require_session(a, s, true)?;
        if !self.run_group_hooks(&g, a, s, "read", true, None).await? {
            return Err(denied("read-not-allowed"));
        }
        self.audit_group_read(&g, a, s).await?;
        let mut messages: Vec<_> = g
            .messages
            .values()
            .filter(|m| {
                m.session_id.as_deref() == s
                    && m.session_seq > after
                    && (g.capability(&a.did, "group.read_all") || g.visible_at(&a.did, m))
            })
            .collect();
        messages.sort_by_key(|m| m.session_seq);
        let more = messages.len() > limit.clamp(1, 4096);
        messages.truncate(limit.clamp(1, 4096));
        let items: Vec<_> = messages
            .iter()
            .map(|m| json!({"seq":m.session_seq,"obj_id":m.obj_id,"redacted":m.redacted,"accepted_at_ms":m.accepted_at_ms}))
            .collect();
        Ok(
            json!({"items":items,"next_after_seq":messages.last().map(|m|m.session_seq).unwrap_or(after),"limited":more}),
        )
    }
    pub(crate) async fn group_changes(
        &self,
        a: &GroupActor,
        group: &DID,
        since: Option<&str>,
        limit: usize,
    ) -> Result<Value> {
        let g = self.groups.load(group).await?.ok_or_else(missing)?;
        g.check_client(a)?;
        let known = g.changes.iter().any(|c| {
            c.data["action"] == "entity.group_deleted" && c.audience.contains(&a.did.to_string())
        }) || g.changes.iter().any(|c| {
            c.data["action"] == "session.deleted" && c.audience.contains(&a.did.to_string())
        }) || g.members.contains_key(&a.did.to_string())
            || g.tombstone_readers.contains(&a.did.to_string())
            || g.participants
                .values()
                .any(|p| p.contains_key(&a.did.to_string()))
            || g.changes.iter().any(|c| c.subject.as_ref() == Some(&a.did));
        if !known {
            return Err(missing());
        }
        let cursor = match since {
            Some(t) => self.groups.cursor(t, group, &a.did).await?,
            None => Some(0),
        };
        let limited = cursor.is_none();
        let after = cursor.unwrap_or(0);
        let mut hooks = BTreeMap::<String, bool>::new();
        let mut items = vec![];
        let mut position = after;
        for c in g.changes.iter().filter(|c| c.group_seq > after) {
            position = c.group_seq;
            let own = c.subject.as_ref() == Some(&a.did);
            let session = c.session_id.as_deref();
            let readable = g.require_session(a, session, true).is_ok();
            let hook = if readable {
                let key = session.unwrap_or("").to_owned();
                if !hooks.contains_key(&key) {
                    hooks.insert(
                        key.clone(),
                        self.run_group_hooks(&g, a, session, "read", true, None)
                            .await
                            .unwrap_or(false),
                    );
                }
                hooks[&key]
            } else {
                false
            };
            let obj_id = c.data["message"]["obj_id"]
                .as_str()
                .and_then(|s| ObjId::new(s).ok());
            let meta = obj_id
                .as_ref()
                .and_then(|id| g.messages.get(&id.to_string()));
            let visible = readable
                && hook
                && (g.capability(&a.did, "group.read_all")
                    || meta.is_some_and(|m| g.visible_at(&a.did, m)));
            if visible {
                items.push(json!({"kind":c.kind,"session":session_key(group,session)?,"session_id":session,"message":{"seq":c.data["message"]["seq"],"obj_id":c.data["message"]["obj_id"],"redacted":meta.is_some_and(|m|m.redacted),"accepted_at_ms":meta.map(|m|m.accepted_at_ms)},"change":c.data["change"],"action":c.data["action"],"subject_did":c.subject}));
            } else if own
                || (c.data["action"] == "entity.group_deleted"
                    && (c.audience.contains(&a.did.to_string())
                        || g.tombstone_readers.contains(&a.did.to_string())))
                || (c.kind == "session"
                    && c.audience.contains(&a.did.to_string())
                    && c.data["action"] == "session.deleted")
            {
                items.push(json!({"kind":if session.is_some(){"session"}else{"member"},"session":session_key(group,session)?,"session_id":session,"change":c.data["change"],"action":c.data["action"],"subject_did":c.subject}));
            }
            if items.len() >= limit.clamp(1, 4096) {
                break;
            }
        }
        let token = self
            .groups
            .save_cursor(group, &a.did, position, Self::now_ms())
            .await?;
        Ok(json!({"items":items,"next_token":token,"limited":limited}))
    }
    async fn audit_group_read(
        &self,
        g: &GroupState,
        a: &GroupActor,
        s: Option<&str>,
    ) -> Result<()> {
        if a.did != g.owner && !g.effective(s, &a.did) && g.capability(&a.did, "group.read_all") {
            let mut tx = self.groups.begin(&g.group_did).await?;
            tx.state.audit.push(json!({"actor":a.did,"session":session_key(&g.group_did,s)?,"action":"group.read_all","at_ms":Self::now_ms()}));
            tx.commit().await?;
        }
        Ok(())
    }
    pub(crate) async fn authorize_group_message(&self, a: &GroupActor, id: &ObjId) -> Result<()> {
        let (group, body, _) = self.groups.object(id).await?.ok_or_else(missing)?;
        if body.is_none() {
            return Err(missing());
        }
        let g = self.groups.load(&group).await?.ok_or_else(missing)?;
        let meta = g.messages.get(&id.to_string()).ok_or_else(missing)?;
        g.require_session(a, meta.session_id.as_deref(), true)?;
        if !g.readable(&a.did, meta) {
            return Err(missing());
        }
        if !self
            .run_group_hooks(&g, a, meta.session_id.as_deref(), "read", true, None)
            .await?
        {
            return Err(missing());
        }
        self.audit_group_read(&g, a, meta.session_id.as_deref())
            .await
    }
    pub(crate) async fn filter_group_records(
        &self,
        a: &GroupActor,
        records: Vec<MailboxRecord>,
    ) -> Result<Vec<MailboxRecord>> {
        let mut output = vec![];
        let mut hooks = BTreeMap::new();
        for record in records {
            let Some(g) = self.groups.load(&record.owner).await? else {
                continue;
            };
            let Some(meta) = g.messages.get(&record.msg_id.to_string()) else {
                continue;
            };
            if g.require_session(a, meta.session_id.as_deref(), true)
                .is_err()
                || (!g.capability(&a.did, "group.read_all") && !g.visible_at(&a.did, meta))
            {
                continue;
            }
            let key = session_key(&g.group_did, meta.session_id.as_deref())?;
            if !hooks.contains_key(&key) {
                hooks.insert(
                    key.clone(),
                    self.run_group_hooks(&g, a, meta.session_id.as_deref(), "read", true, None)
                        .await
                        .unwrap_or(false),
                );
            }
            if hooks[&key] {
                output.push(record);
            }
        }
        Ok(output)
    }

    pub(crate) async fn post_hosted_group(
        &self,
        a: &GroupActor,
        msg: MsgObject,
    ) -> Result<buckyos_api::PostSendResult> {
        let result = self.accept_group_message(a, msg, None).await?;
        Ok(buckyos_api::PostSendResult {
            ok: result.ok,
            msg_id: result.msg_id,
            deliveries: vec![],
            reason: result.reason,
        })
    }
    pub(crate) async fn accept_group_message(
        &self,
        a: &GroupActor,
        msg: MsgObject,
        jwt: Option<String>,
    ) -> Result<DispatchResult> {
        if msg.from != a.did {
            return Err(denied("sender-mismatch"));
        }
        Self::validate_ingress_message(&msg)?;
        if msg.kind != MsgObjKind::GroupMsg || msg.to.len() != 1 {
            return Err(invalid("group-message-requires-single-group-target"));
        }
        let group = msg.to[0].clone();
        let id = msg.gen_obj_id().0;
        let mut tx = self.groups.begin(&group).await?;
        let key = format!("message:{}", id.to_string());
        if let Some(saved) = tx.state.operations.get(&key) {
            return serde_json::from_value(saved.result.clone()).map_err(db_error);
        }
        let result = self
            .accept_group_message_tx(&mut tx, a, &msg, jwt.as_deref())
            .await;
        let result = match result {
            Ok(meta) => DispatchResult {
                ok: true,
                msg_id: id.clone(),
                delivered_recipients: vec![],
                dropped_recipients: vec![],
                delivered_group: Some(group),
                delivered_agents: vec![],
                reason: Some(format!("accepted:session_seq:{}", meta.session_seq)),
            },
            Err(kRPC::RPCErrors::NoPermission(reason))
            | Err(kRPC::RPCErrors::ParseRequestError(reason)) => DispatchResult {
                ok: false,
                msg_id: id.clone(),
                delivered_recipients: vec![],
                dropped_recipients: vec![],
                delivered_group: None,
                delivered_agents: vec![],
                reason: Some(reason),
            },
            Err(e) => return Err(e),
        };
        tx.state.operations.insert(
            key,
            SavedOperation {
                request: json!({"obj_id":id,"from":a.did}),
                result: encoded(&result)?,
            },
        );
        tx.commit().await?;
        if let Some(group) = &result.delivered_group {
            Self::publish_event(
                format!("/msg_center/group/{}/changed", group.to_string()),
                json!({"group_did":group}),
            );
        }
        Ok(result)
    }
    async fn accept_group_message_tx(
        &self,
        tx: &mut GroupTransaction<'_>,
        a: &GroupActor,
        msg: &MsgObject,
        jwt: Option<&str>,
    ) -> Result<MessageMeta> {
        let s = msg.to_session.as_deref();
        tx.state.require_session(a, s, false)?;
        if tx.state.lifecycle != "active"
            || s.is_some_and(|s| tx.state.sessions[s].lifecycle != "active")
        {
            return Err(denied("session-archived"));
        }
        let rules = tx.state.rules(s)?;
        let config = tx.state.config()?;
        if msg.gen_obj_id().1.len() > config.limits.max_message_bytes {
            return Err(invalid("message-too-large"));
        }
        if let Some(jwt) = jwt {
            let signed = crate::cyfs_dispatch::verify_signed_message(jwt)
                .await
                .map_err(|(_, reason)| denied(reason))?;
            if signed.msg != *msg {
                return Err(denied("signed-message-mismatch"));
            }
        }
        let mut redact = vec![];
        if let Some(relation) = &msg.relates_to {
            let target = tx
                .state
                .messages
                .get(&relation.target.to_string())
                .filter(|m| m.session_id == msg.to_session && !m.redacted)
                .ok_or_else(missing)?;
            if !tx.state.readable(&a.did, target) {
                return Err(missing());
            }
            match relation.rel {
                MsgRelType::Edit => {
                    if target.from != a.did
                        || target.kind != MsgObjKind::GroupMsg
                        || target.relation.is_some()
                        || !within(
                            rules.edit.edit_window_ms,
                            target.accepted_at_ms,
                            Self::now_ms(),
                        )
                    {
                        return Err(denied("edit-not-allowed"));
                    }
                }
                MsgRelType::Redact => {
                    if !(tx.state.capability(&a.did, "message.redact_any")
                        || (target.from == a.did
                            && within(
                                rules.edit.recall_window_ms,
                                target.accepted_at_ms,
                                Self::now_ms(),
                            )))
                    {
                        return Err(denied("redact-not-allowed"));
                    }
                    redact.push(target.obj_id.clone());
                    for m in tx.state.messages.values() {
                        if m.relation.as_ref().is_some_and(|(rel, id, _)| {
                            *id == target.obj_id
                                && matches!(rel, MsgRelType::Edit | MsgRelType::Reaction)
                        }) {
                            redact.push(m.obj_id.clone());
                        }
                    }
                }
                MsgRelType::Reaction => {
                    if let Some(existing) = tx.state.messages.values().find(|m| {
                        !m.redacted
                            && m.from == a.did
                            && m.relation.as_ref().is_some_and(|(rel, id, key)| {
                                *rel == MsgRelType::Reaction
                                    && *id == relation.target
                                    && *key == relation.key
                            })
                    }) {
                        return Ok(existing.clone());
                    }
                }
                _ => {}
            }
        }
        if msg.mentions.as_ref().is_some_and(|m| m.all)
            && !tx.state.capability(&a.did, "session.mention_all")
        {
            return Err(denied("mention-all-not-allowed"));
        }
        let agent = tx
            .state
            .members
            .get(&a.did.to_string())
            .is_some_and(|m| m.entity_kind == "agent");
        Self::rate(&mut tx.state, &a.did, s.unwrap_or(""), agent)?;
        let role = tx.state.role(&a.did);
        let admin = matches!(role, Some(GroupRole::Owner | GroupRole::Admin));
        if !admin
            && rules.slow_mode_ms.is_some_and(|delay| {
                tx.state.messages.values().any(|m| {
                    m.session_id == msg.to_session
                        && m.from == a.did
                        && Self::now_ms().saturating_sub(m.accepted_at_ms) < delay as u64
                })
            })
        {
            return Err(kRPC::RPCErrors::ReasonError("slow-mode".into()));
        }
        let muted = tx
            .state
            .moderation
            .get(&a.did.to_string())
            .is_some_and(|m| m.muted_until_ms.is_some_and(|t| t > Self::now_ms()));
        let native = tx.state.native_post_allowed(
            a,
            s,
            msg.relates_to
                .as_ref()
                .is_some_and(|r| r.rel == MsgRelType::Reaction),
        )?;
        if !self
            .run_group_hooks(&tx.state, a, s, "post", native, Some(msg))
            .await?
        {
            return Err(denied(if muted { "muted" } else { "post-not-allowed" }));
        }
        let seq = Self::allocate(&mut tx.state, s);
        let meta = self
            .append_group_message(
                tx,
                msg,
                jwt,
                seq,
                "message",
                None,
                json!({}),
                BTreeSet::new(),
            )
            .await?;
        for id in redact {
            tx.redact(&id).await?;
        }
        Ok(meta)
    }
}

impl MessageCenter {
    pub(crate) async fn authorize_group_mailbox(
        &self,
        a: &GroupActor,
        mailbox: &MailboxAddress,
    ) -> Result<()> {
        let g = self
            .groups
            .load(mailbox.owner())
            .await?
            .ok_or_else(missing)?;
        g.require_session(a, mailbox.session_id(), true)?;
        if !self
            .run_group_hooks(&g, a, mailbox.session_id(), "read", true, None)
            .await?
        {
            return Err(missing());
        }
        self.audit_group_read(&g, a, mailbox.session_id()).await
    }
    pub(crate) async fn list_group_mailbox(
        &self,
        a: &GroupActor,
        mailbox: MailboxAddress,
        state: Option<Vec<RecipientState>>,
        limit: Option<usize>,
        cursor_sort: Option<u64>,
        cursor_id: Option<String>,
        descending: bool,
        with_object: Option<bool>,
    ) -> Result<buckyos_api::MailboxRecordPage> {
        let records = self
            .msg_box_db
            .list_records(
                &mailbox,
                &MailboxKind::GroupInbox,
                state.as_deref(),
                descending,
            )
            .await?;
        let records =
            Self::filter_after_cursor(records, cursor_sort, cursor_id.as_deref(), descending);
        let records = self.filter_group_records(a, records).await?;
        let limit = limit.unwrap_or(100).clamp(1, 4096);
        let more = records.len() > limit;
        let mut items = vec![];
        for record in records.into_iter().take(limit) {
            items.push(self.build_record_view(record, with_object).await?);
        }
        let next = if more {
            items
                .last()
                .map(|i| (i.record.sort_key, i.record.record_id.clone()))
        } else {
            None
        };
        Ok(buckyos_api::MailboxRecordPage {
            items,
            next_cursor_sort_key: next.as_ref().map(|(s, _)| *s),
            next_cursor_record_id: next.map(|(_, id)| id),
        })
    }
}

impl MessageCenter {
    pub(crate) async fn authorize_attachment(
        &self,
        ctx: &RPCContext,
        id: &ObjId,
        context_path: Option<&str>,
    ) -> Result<()> {
        if let Some(path) = context_path {
            let actor = self.group_actor(ctx).await?;
            return self.authorize_group_attachment(&actor, id, path).await;
        }
        let actor = self.group_actor(ctx).await?;
        use sqlx::Row;
        let sql = self
            .msg_box_db
            .render_sql("SELECT msg_id FROM msg_refs WHERE owner=?");
        for row in sqlx::query(&sql)
            .bind(actor.did.to_string())
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(db_error)?
        {
            let raw: String = row.try_get("msg_id").map_err(db_error)?;
            let msg_id = ObjId::new(&raw).map_err(db_error)?;
            if self.authorize_message(ctx, &msg_id).await.is_err() {
                continue;
            }
            if let Ok(message) = self.load_message(&msg_id).await {
                if Self::message_references(&message, id) {
                    return Ok(());
                }
            }
        }
        Err(missing())
    }
    pub(crate) async fn authorize_group_attachment(
        &self,
        actor: &GroupActor,
        id: &ObjId,
        path: &str,
    ) -> Result<()> {
        let (mailbox, msg) = path.rsplit_once('/').ok_or_else(missing)?;
        let mailbox = MailboxAddress::try_from(mailbox.to_string()).map_err(|_| missing())?;
        let msg_id = ObjId::new(msg).map_err(|_| missing())?;
        self.authorize_group_message(actor, &msg_id).await?;
        let (group, body, _) = self.groups.object(&msg_id).await?.ok_or_else(missing)?;
        let message: MsgObject =
            serde_json::from_str(&body.ok_or_else(missing)?).map_err(db_error)?;
        if group != *mailbox.owner()
            || message.to_session.as_deref() != mailbox.session_id()
            || !Self::message_references(&message, id)
        {
            return Err(missing());
        }
        Ok(())
    }
    pub(crate) fn message_references(msg: &MsgObject, id: &ObjId) -> bool {
        msg.content
            .refs
            .iter()
            .any(|r| matches!(&r.target,ndn_lib::RefTarget::DataObj{obj_id,..} if obj_id==id))
    }
}

impl MessageCenter {
    pub(crate) async fn advance_group_read_from_record(
        &self,
        record: &MailboxRecord,
    ) -> Result<()> {
        if record.box_kind != MailboxKind::Inbox {
            return Ok(());
        }
        let Some(group) = record
            .tags
            .iter()
            .find_map(|t| t.strip_prefix("group:").and_then(|d| DID::from_str(d).ok()))
        else {
            return Ok(());
        };
        if let Some(g) = self.groups.load(&group).await? {
            let Some(meta) = g.messages.get(&record.msg_id.to_string()) else {
                return Ok(());
            };
            if !g.effective(meta.session_id.as_deref(), &record.owner) {
                return Ok(());
            }
            let mut tx = self.groups.begin(&group).await?;
            let marker = tx
                .state
                .read_markers
                .entry(meta.session_id.clone().unwrap_or_default())
                .or_default()
                .entry(record.owner.to_string())
                .or_default();
            *marker = (*marker).max(meta.session_seq);
            tx.commit().await?;
        } else {
            self.advance_joined_read_from_record(record, &group).await?;
        }
        Ok(())
    }
}
