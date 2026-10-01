use buckyos_api::MailboxAddress;
use kRPC::RPCErrors;
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgRelType, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub type Result<T> = std::result::Result<T, RPCErrors>;
pub fn invalid(reason: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ParseRequestError(reason.to_string())
}
pub fn denied(reason: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::NoPermission(reason.to_string())
}
pub fn missing() -> RPCErrors {
    denied("not-found")
}
pub fn revision() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn session_key(group: &DID, session: Option<&str>) -> Result<String> {
    MailboxAddress::new(group.clone(), session.map(str::to_owned))
        .map(|m| m.to_string())
        .map_err(invalid)
}
pub fn validate_sid(s: &str) -> Result<()> {
    MailboxAddress::validate_session_id(s).map_err(invalid)?;
    if s.starts_with('_') {
        return Err(invalid("reserved-session-id"));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct GroupActor {
    pub did: DID,
    pub client: Option<String>,
    pub remote: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupRole {
    Owner,
    Admin,
    Member,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Invited,
    PendingAdminApproval,
    Active,
    Left,
    Removed,
    Rejected,
    Expired,
    Revoked,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberRecord {
    pub member_did: DID,
    pub role: GroupRole,
    pub state: MemberStatus,
    pub epoch: u32,
    pub entity_kind: String,
    pub invitation_id: Option<String>,
    /// Who issued the current invitation; decides whether acceptance still
    /// needs admin approval (member-issued invitations do).
    #[serde(default)]
    pub invited_by: Option<DID>,
    pub expires_at_ms: Option<u64>,
    pub since_seq: u64,
}
/// An owner transfer offered by the controller and not yet accepted by the
/// target member.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnerTransfer {
    pub member_did: DID,
    pub transfer_id: String,
    pub expires_at_ms: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Moderation {
    pub blocked: bool,
    pub muted_until_ms: Option<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMembership {
    #[default]
    Inherit,
    Roles(Vec<GroupRole>),
    Explicit,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostRule {
    AllParticipants,
    Only(Vec<String>),
    Nobody,
}
impl Default for PostRule {
    fn default() -> Self {
        Self::AllParticipants
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EditRule {
    pub edit_window_ms: Option<u64>,
    pub recall_window_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookBinding {
    pub point: String,
    pub service: String,
    #[serde(default)]
    pub may_grant: bool,
    pub on_unavailable: String,
    pub timeout_ms: u32,
    #[serde(default)]
    pub result_ttl_ms: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionRules {
    pub post: PostRule,
    pub react: PostRule,
    pub history: String,
    pub allow_guests: bool,
    pub edit: EditRule,
    pub receipts: String,
    pub slow_mode_ms: Option<u32>,
    pub hooks: Vec<HookBinding>,
    pub panels: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
impl Default for SessionRules {
    fn default() -> Self {
        Self {
            post: PostRule::default(),
            react: PostRule::default(),
            history: "from_join".into(),
            allow_guests: false,
            edit: EditRule::default(),
            receipts: "hidden".into(),
            slow_mode_ms: None,
            hooks: vec![],
            panels: vec![],
            extra: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionTemplate {
    pub membership: SessionMembership,
    pub rules: SessionRules,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupLimits {
    pub max_members: usize,
    pub max_sessions: usize,
    pub max_guests_per_session: usize,
    pub max_message_bytes: usize,
    pub receipts_max_members: usize,
    pub messages_per_window: usize,
    pub agent_messages_per_window: usize,
    pub rate_window_ms: u64,
}
impl Default for GroupLimits {
    fn default() -> Self {
        Self {
            max_members: 500,
            max_sessions: 10_000,
            max_guests_per_session: 20,
            max_message_bytes: 65_536,
            receipts_max_members: 100,
            messages_per_window: 20,
            agent_messages_per_window: 10,
            rate_window_ms: 10_000,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuestEntry {
    pub session_template: String,
    pub max_open_per_guest: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MembershipConfig {
    pub join_policy: String,
    pub member_list_visibility: String,
    pub guest_entry: Option<GuestEntry>,
}
impl Default for MembershipConfig {
    fn default() -> Self {
        Self {
            join_policy: "invite_only".into(),
            member_list_visibility: "members".into(),
            guest_entry: None,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowedClients {
    #[default]
    Any,
    Only(Vec<String>),
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccessConfig {
    pub allowed_clients: AllowedClients,
    pub allow_unverified_remote: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupConfiguration {
    pub schema_version: u32,
    pub required_features: Vec<String>,
    pub revision: String,
    pub profile: Value,
    pub membership: MembershipConfig,
    pub roles: BTreeMap<String, BTreeSet<String>>,
    pub default_session: SessionRules,
    pub session_templates: BTreeMap<String, SessionTemplate>,
    pub limits: GroupLimits,
    pub display: Value,
    pub access: AccessConfig,
    pub extensions: Value,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
pub const OWNER_CAPS: &[&str] = &[
    "group.update_profile",
    "group.update_config",
    "group.invite_member",
    "group.approve_member",
    "group.remove_member",
    "group.moderate",
    "group.update_role",
    "group.read_all",
    "message.redact_any",
    "session.create",
    "session.manage",
    "session.invite_guest",
    "session.update_shared_state",
    "session.mention_all",
    "session.post",
    "session.read",
];
impl Default for GroupConfiguration {
    fn default() -> Self {
        let mut roles = BTreeMap::new();
        roles.insert(
            "owner".into(),
            OWNER_CAPS.iter().map(|s| s.to_string()).collect(),
        );
        roles.insert(
            "admin".into(),
            OWNER_CAPS
                .iter()
                .filter(|s| !matches!(**s, "group.update_role" | "group.read_all"))
                .map(|s| s.to_string())
                .collect(),
        );
        roles.insert(
            "member".into(),
            ["session.post", "session.read"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        );
        Self {
            schema_version: 1,
            required_features: vec![],
            revision: revision(),
            profile: json!({}),
            membership: MembershipConfig::default(),
            roles,
            default_session: SessionRules::default(),
            session_templates: BTreeMap::new(),
            limits: GroupLimits::default(),
            display: json!({}),
            access: AccessConfig::default(),
            extensions: json!({}),
            extra: BTreeMap::new(),
        }
    }
}
impl GroupConfiguration {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("unsupported-schema-version"));
        }
        for f in &self.required_features {
            if ![
                "sessions",
                "session_guests",
                "message_relations",
                "read_markers",
                "hooks",
                "allowed_clients",
            ]
            .contains(&f.as_str())
            {
                return Err(invalid(format!("unsupported-required-feature:{f}")));
            }
        }
        if !["invite_only", "request_and_approve", "open"]
            .contains(&self.membership.join_policy.as_str())
            || !["members", "admins_only", "public"]
                .contains(&self.membership.member_list_visibility.as_str())
        {
            return Err(invalid("invalid-membership-config"));
        }
        let max = GroupLimits::default();
        let l = &self.limits;
        if l.max_members == 0
            || l.max_members > max.max_members
            || l.max_sessions == 0
            || l.max_sessions > max.max_sessions
            || l.max_guests_per_session > max.max_guests_per_session
            || l.max_message_bytes == 0
            || l.max_message_bytes > max.max_message_bytes
            || l.receipts_max_members > max.receipts_max_members
            || l.messages_per_window == 0
            || l.messages_per_window > max.messages_per_window
            || l.agent_messages_per_window == 0
            || l.agent_messages_per_window > max.agent_messages_per_window
            || l.rate_window_ms < max.rate_window_ms
        {
            return Err(invalid("invalid-group-limits"));
        }
        validate_rules(&SessionMembership::Inherit, &self.default_session, true)?;
        for (id, t) in &self.session_templates {
            validate_sid(id)?;
            validate_rules(&t.membership, &t.rules, false)?;
        }
        if let Some(e) = &self.membership.guest_entry {
            let t = self
                .session_templates
                .get(&e.session_template)
                .ok_or_else(|| invalid("unknown-guest-template"))?;
            if !t.rules.allow_guests
                || t.membership == SessionMembership::Inherit
                || e.max_open_per_guest == 0
                || e.max_open_per_guest > 20
            {
                return Err(invalid("invalid-guest-entry"));
            }
        }
        Ok(())
    }
}
pub fn validate_rules(m: &SessionMembership, r: &SessionRules, default: bool) -> Result<()> {
    if r.allow_guests && (default || *m == SessionMembership::Inherit) {
        return Err(invalid("guests-require-named-restricted-session"));
    }
    if !["all", "from_join"].contains(&r.history.as_str())
        || !["hidden", "count", "readers"].contains(&r.receipts.as_str())
    {
        return Err(invalid("invalid-session-rules"));
    }
    for p in [&r.post, &r.react] {
        if let PostRule::Only(kinds) = p {
            if kinds
                .iter()
                .any(|k| !["owner", "admin", "member", "guest"].contains(&k.as_str()))
            {
                return Err(invalid("invalid-participant-kind"));
            }
        }
    }
    for h in &r.hooks {
        if !["join", "post", "read"].contains(&h.point.as_str())
            || !["deny", "native_only"].contains(&h.on_unavailable.as_str())
            || h.timeout_ms == 0
            || h.timeout_ms > 30_000
            || (h.point != "post" && h.may_grant)
        {
            return Err(invalid("invalid-hook-binding"));
        }
        let url = reqwest::Url::parse(&h.service).map_err(|_| invalid("invalid-hook-service"))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid("invalid-hook-service"));
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupSessionRecord {
    pub group_did: DID,
    pub session_id: String,
    pub template: Option<String>,
    pub membership: SessionMembership,
    pub rule_overrides: Value,
    pub lifecycle: String,
    pub revision: String,
    pub created_by: DID,
    pub created_at_ms: u64,
    pub shared_state: Value,
    pub member_states: BTreeMap<String, Value>,
    pub guest_request: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMembershipRecord {
    pub group_did: DID,
    pub session_id: Option<String>,
    pub member_did: DID,
    pub kind: String,
    pub epoch: u32,
    pub state: String,
    pub since_seq: u64,
    pub actor: DID,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisibleInterval {
    pub from_group_seq: u64,
    pub to_group_seq: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupInviteLink {
    pub token: String,
    pub created_by: DID,
    pub expires_at_ms: Option<u64>,
    pub max_uses: Option<u32>,
    pub used: u32,
    pub require_approval: bool,
    pub revoked: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageMeta {
    pub obj_id: ObjId,
    pub session_id: Option<String>,
    pub group_seq: u64,
    pub session_seq: u64,
    pub accepted_at_ms: u64,
    pub from: DID,
    pub kind: MsgObjKind,
    pub relation: Option<(MsgRelType, ObjId, Option<String>)>,
    pub redacted: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub group_seq: u64,
    pub kind: String,
    pub session_id: Option<String>,
    pub subject: Option<DID>,
    pub data: Value,
    pub audience: BTreeSet<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedOperation {
    pub request: Value,
    pub result: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupState {
    pub schema_version: u32,
    pub group_did: DID,
    pub controller: DID,
    pub owner: DID,
    pub host: DID,
    pub lifecycle: String,
    pub configuration: Value,
    #[serde(default)]
    pub doc_updated_at_ms: u64,
    #[serde(default)]
    pub config_history: BTreeMap<String, Value>,
    pub members: BTreeMap<String, MemberRecord>,
    pub sessions: BTreeMap<String, GroupSessionRecord>,
    pub participants: BTreeMap<String, BTreeMap<String, SessionMembershipRecord>>,
    pub intervals: BTreeMap<String, BTreeMap<String, Vec<VisibleInterval>>>,
    pub moderation: BTreeMap<String, Moderation>,
    #[serde(default)]
    pub pending_owner_transfer: Option<OwnerTransfer>,
    pub invite_links: BTreeMap<String, GroupInviteLink>,
    pub group_seq: u64,
    pub session_seqs: BTreeMap<String, u64>,
    pub accepted_at_ms: u64,
    pub messages: BTreeMap<String, MessageMeta>,
    pub changes: Vec<Change>,
    pub shared_states: BTreeMap<String, Value>,
    pub member_states: BTreeMap<String, BTreeMap<String, Value>>,
    pub operations: BTreeMap<String, SavedOperation>,
    pub read_markers: BTreeMap<String, BTreeMap<String, u64>>,
    pub rates: BTreeMap<String, Vec<u64>>,
    pub audit: Vec<Value>,
    /// DIDs of former participants who may still read the deletion notice.
    #[serde(default)]
    pub tombstone_readers: BTreeSet<String>,
}
impl GroupState {
    pub fn public_doc(&self) -> Result<Value> {
        if self.lifecycle == "deleted" {
            return Err(missing());
        }
        let c = self.config()?;
        Ok(
            json!({"@context":["https://www.w3.org/ns/did/v1","https://buckyos.org/ns/group/v1"],"obj_type":"buckyos.group_doc","schema_version":1,"id":self.group_did,"entity_type":"group","host":self.host,"controller":self.controller,"owner":self.owner,"service_path":format!("/{}/",self.group_did.to_string()),"profile":c.profile,"join_policy":c.membership.join_policy,"revision":c.revision,"lifecycle":self.lifecycle,"iat":self.doc_updated_at_ms/1000}),
        )
    }
    pub fn config(&self) -> Result<GroupConfiguration> {
        serde_json::from_value(self.configuration.clone()).map_err(|e| invalid(e.to_string()))
    }
    pub fn rules(&self, s: Option<&str>) -> Result<SessionRules> {
        let c = self.config()?;
        let Some(s) = s else {
            return Ok(c.default_session);
        };
        let r = self
            .sessions
            .get(s)
            .filter(|r| r.lifecycle != "deleted")
            .ok_or_else(missing)?;
        let mut rules = serde_json::to_value(match r.template.as_ref() {
            Some(t) => c
                .session_templates
                .get(t)
                .ok_or_else(|| invalid("missing-template"))?
                .rules
                .clone(),
            None => SessionRules::default(),
        })
        .map_err(|e| invalid(e.to_string()))?;
        merge(&mut rules, &r.rule_overrides);
        serde_json::from_value(rules).map_err(|e| invalid(e.to_string()))
    }
    pub fn role(&self, d: &DID) -> Option<GroupRole> {
        self.members
            .get(&d.to_string())
            .filter(|m| m.state == MemberStatus::Active)
            .map(|m| m.role)
    }
    pub fn blocked(&self, d: &DID) -> bool {
        self.moderation
            .get(&d.to_string())
            .is_some_and(|m| m.blocked)
    }
    pub fn capability(&self, d: &DID, cap: &str) -> bool {
        if self.blocked(d) {
            return false;
        }
        let Some(role) = self.role(d) else {
            return false;
        };
        if role == GroupRole::Owner {
            return OWNER_CAPS.contains(&cap);
        }
        self.config()
            .ok()
            .and_then(|c| {
                c.roles
                    .get(match role {
                        GroupRole::Owner => "owner",
                        GroupRole::Admin => "admin",
                        GroupRole::Member => "member",
                    })
                    .cloned()
            })
            .is_some_and(|caps| caps.contains(cap))
    }
    pub fn effective(&self, s: Option<&str>, d: &DID) -> bool {
        if self.lifecycle == "deleted" || self.blocked(d) {
            return false;
        }
        let sid = s.unwrap_or("");
        let record = self
            .participants
            .get(sid)
            .and_then(|p| p.get(&d.to_string()));
        let membership = match s {
            None => SessionMembership::Inherit,
            Some(s) => match self.sessions.get(s).filter(|r| r.lifecycle != "deleted") {
                Some(r) => r.membership.clone(),
                None => return false,
            },
        };
        if let Some(m) = self
            .members
            .get(&d.to_string())
            .filter(|m| m.state == MemberStatus::Active)
        {
            let r = record.filter(|r| r.kind == "group_member" && r.epoch == m.epoch);
            return match membership {
                SessionMembership::Inherit => {
                    !r.is_some_and(|r| matches!(r.state.as_str(), "left" | "removed"))
                }
                SessionMembership::Roles(roles) => {
                    roles.contains(&m.role)
                        && !r.is_some_and(|r| matches!(r.state.as_str(), "left" | "removed"))
                }
                SessionMembership::Explicit => r.is_some_and(|r| r.state == "included"),
            };
        }
        s.is_some()
            && self.rules(s).is_ok_and(|r| r.allow_guests)
            && record.is_some_and(|r| r.kind == "guest" && r.state == "included")
    }
    pub fn candidates(&self) -> BTreeSet<String> {
        self.members
            .keys()
            .chain(self.participants.values().flat_map(|p| p.keys()))
            .cloned()
            .collect()
    }
    pub fn audience(&self, s: Option<&str>) -> BTreeSet<String> {
        self.candidates()
            .into_iter()
            .filter(|d| DID::from_str(d).is_ok_and(|d| self.effective(s, &d)))
            .collect()
    }
    pub fn readable(&self, d: &DID, m: &MessageMeta) -> bool {
        if m.redacted || self.lifecycle == "deleted" {
            return false;
        }
        if self.capability(d, "group.read_all") {
            return self.rules(m.session_id.as_deref()).is_ok();
        }
        self.effective(m.session_id.as_deref(), d) && self.visible_at(d, m)
    }
    pub fn visible_at(&self, d: &DID, m: &MessageMeta) -> bool {
        if self
            .rules(m.session_id.as_deref())
            .is_ok_and(|r| r.history == "all")
        {
            return true;
        }
        self.intervals
            .get(m.session_id.as_deref().unwrap_or(""))
            .and_then(|v| v.get(&d.to_string()))
            .is_some_and(|v| {
                v.iter().any(|i| {
                    m.group_seq >= i.from_group_seq
                        && i.to_group_seq.is_none_or(|end| m.group_seq < end)
                })
            })
    }
    pub fn refresh_intervals(&mut self, seq: u64) {
        let sessions: Vec<_> = std::iter::once(String::new())
            .chain(self.sessions.keys().cloned())
            .collect();
        let candidates = self.candidates();
        for s in sessions {
            for d in &candidates {
                let effective = DID::from_str(d)
                    .is_ok_and(|d| self.effective(if s.is_empty() { None } else { Some(&s) }, &d));
                let intervals = self
                    .intervals
                    .entry(s.clone())
                    .or_default()
                    .entry(d.clone())
                    .or_default();
                let open = intervals.last().is_some_and(|i| i.to_group_seq.is_none());
                if effective && !open {
                    intervals.push(VisibleInterval {
                        from_group_seq: seq,
                        to_group_seq: None,
                    });
                } else if !effective && open {
                    intervals.last_mut().unwrap().to_group_seq = Some(seq);
                }
            }
        }
    }
    pub fn require_session(&self, a: &GroupActor, s: Option<&str>, read: bool) -> Result<()> {
        if !self.effective(s, &a.did) && !(read && self.capability(&a.did, "group.read_all")) {
            return Err(missing());
        }
        self.rules(s)?;
        if read
            && self.role(&a.did).is_some()
            && !self.capability(&a.did, "session.read")
            && !self.capability(&a.did, "group.read_all")
        {
            return Err(denied("read-not-allowed"));
        }
        self.check_client(a)
    }
    pub fn native_post_allowed(
        &self,
        a: &GroupActor,
        s: Option<&str>,
        reaction: bool,
    ) -> Result<bool> {
        let rules = self.rules(s)?;
        let role = self.role(&a.did);
        let participant = match role {
            Some(GroupRole::Owner) => "owner",
            Some(GroupRole::Admin) => "admin",
            Some(GroupRole::Member) => "member",
            None => "guest",
        };
        let rule = if reaction { &rules.react } else { &rules.post };
        let allowed = match rule {
            PostRule::AllParticipants => true,
            PostRule::Only(kinds) => kinds.iter().any(|k| k == participant),
            PostRule::Nobody => false,
        };
        let muted = self.moderation.get(&a.did.to_string()).is_some_and(|m| {
            m.muted_until_ms
                .is_some_and(|t| t > crate::msg_center::MessageCenter::now_ms())
        });
        Ok(allowed && !muted && (role.is_none() || self.capability(&a.did, "session.post")))
    }
    pub fn check_client(&self, a: &GroupActor) -> Result<()> {
        match self.config()?.access {
            AccessConfig {
                allowed_clients: AllowedClients::Any,
                ..
            } => Ok(()),
            AccessConfig {
                allowed_clients: AllowedClients::Only(clients),
                allow_unverified_remote,
            } => {
                if (a.remote && allow_unverified_remote)
                    || (!a.remote && a.client.as_ref().is_some_and(|c| clients.contains(c)))
                {
                    Ok(())
                } else {
                    Err(denied("client-not-allowed"))
                }
            }
        }
    }
    pub fn require_cap(&self, a: &GroupActor, cap: &str, s: Option<&str>) -> Result<()> {
        if self.role(&a.did).is_none() || self.blocked(&a.did) || self.lifecycle == "deleted" {
            return Err(missing());
        }
        self.check_client(a)?;
        if s.is_some() && !self.effective(s, &a.did) && !self.capability(&a.did, "group.read_all") {
            return Err(missing());
        }
        let own_session = s
            .and_then(|s| self.sessions.get(s))
            .is_some_and(|r| r.created_by == a.did);
        if self.capability(&a.did, cap)
            || (own_session && matches!(cap, "session.manage" | "session.invite_guest"))
        {
            Ok(())
        } else {
            Err(denied("capability-denied"))
        }
    }
}
pub fn merge(target: &mut Value, patch: &Value) {
    if let (Some(t), Some(p)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            if v.is_null() {
                t.remove(k);
            } else {
                merge(t.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
    } else {
        *target = patch.clone();
    }
}
