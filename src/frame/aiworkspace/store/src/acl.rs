//! Permissions of one Workspace (权限需求 §4): entries that allow and deny capabilities to a subject on a resource,
//! the Workspace's logical groups and their members, the fixed groups (`@owner`, `@all`, `@guest`, `@creator`), the
//! anonymous access switch, macro bindings and the operation record. All of it is service control state in
//! `local.sqlite`: never in the document's history, never undone with content (§4.3, Q3).
//!
//! Interfaces address resources by the core API's paths (R1); storage binds entity ids (`""` = the Workspace, `/`).

use crate::docdb::{db_err, meta_get};
use crate::workspace::{Caller, CallerClass, MemberMatch, Workspace, ANY_ACCOUNT_MEMBER, CONTACT_PREFIX, GROUP_PREFIX};
use aiworkspace_core::access::{cap_name, Access, Applied, Cap, CapSet, Tier, ALL_CAPS};
use aiworkspace_core::model::{ReadCtx, ROOT_ID, SCOPE_SHARED, TYPE_WISH};
use aiworkspace_core::{Code, Deny, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const SUBJECT_OWNER: &str = "@owner";
pub const SUBJECT_ALL: &str = "@all";
pub const SUBJECT_GUEST: &str = "@guest";
pub const SUBJECT_CREATOR: &str = "@creator";
/// `@pub:<publication id>`: an inner-path publication's own subject (R26), `read` only.
pub const PUB_PREFIX: &str = "@pub:";
const RESERVED_GROUP_IDS: &[&str] = &["owner", "all", "guest", "creator", "pub"];
const MAX_CHANGES: usize = 500;
const MAX_GROUPS: usize = 200;
const MAX_MEMBERS: usize = 5000;

#[derive(Debug, Clone)]
pub struct EntryRow {
    pub subject: String,
    /// `""` = the Workspace.
    pub scope: String,
    pub allow: CapSet,
    pub deny: CapSet,
    pub updated_by: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct GroupRow {
    pub group_id: String,
    pub title: String,
    pub description: String,
    pub created_by: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct MemberRow {
    pub group_id: String,
    pub member: String,
    pub via_entry: Option<String>,
    pub added_by: String,
    pub added_at: String,
}

/// Everything a decision reads, loaded per request (closing an entry or removing a member takes effect at once).
#[derive(Debug, Clone, Default)]
pub struct AclData {
    pub owner: Option<String>,
    pub anonymous: bool,
    pub entries: Vec<EntryRow>,
    pub groups: BTreeMap<String, GroupRow>,
    pub members: Vec<MemberRow>,
}

fn rows<T>(conn: &Connection, sql: &str, f: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>) -> WsResult<Vec<T>> {
    let mut st = conn.prepare_cached(sql).map_err(db_err)?;
    let out = st.query_map([], f).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
    Ok(out)
}

impl AclData {
    pub fn load(conn: &Connection) -> WsResult<AclData> {
        let meta = |k: &str| -> WsResult<Option<String>> { conn.query_row("SELECT value FROM local_meta WHERE key = ?1", [k], |r| r.get(0)).optional().map_err(db_err) };
        let entries = rows(conn, "SELECT subject, scope_entity_id, allow, deny, updated_by, updated_at FROM acl_entries ORDER BY scope_entity_id, subject", |r| {
            Ok(EntryRow { subject: r.get(0)?, scope: r.get(1)?, allow: CapSet(r.get(2)?), deny: CapSet(r.get(3)?), updated_by: r.get(4)?, updated_at: r.get(5)? })
        })?;
        let groups = rows(conn, "SELECT group_id, title, description, created_by, created_at FROM acl_groups ORDER BY group_id", |r| {
            Ok(GroupRow { group_id: r.get(0)?, title: r.get(1)?, description: r.get(2)?, created_by: r.get(3)?, created_at: r.get(4)? })
        })?
        .into_iter()
        .map(|g| (g.group_id.clone(), g))
        .collect();
        let members = rows(conn, "SELECT group_id, member, via_entry, added_by, added_at FROM acl_members ORDER BY group_id, member", |r| {
            Ok(MemberRow { group_id: r.get(0)?, member: r.get(1)?, via_entry: r.get(2)?, added_by: r.get(3)?, added_at: r.get(4)? })
        })?;
        Ok(AclData { owner: meta("owner")?, anonymous: meta("anonymous")?.as_deref() == Some("1"), entries, groups, members })
    }

    pub fn has_entries(&self, subject: &str) -> bool {
        self.entries.iter().any(|e| e.subject == subject)
    }

    /// What a group's entries allow anywhere.
    pub fn allowed_anywhere(&self, subject: &str) -> CapSet {
        self.entries.iter().filter(|e| e.subject == subject).fold(CapSet::NONE, |c, e| c.union(e.allow))
    }

    /// An Agent is a subject of its own here once anything names its DID (Q29).
    pub fn configures(&self, principal: &str) -> bool {
        self.entries.iter().any(|e| e.subject == principal) || self.members.iter().any(|m| m.member == principal)
    }
}

/// Drop what the permission model replaced (`grants`, entries carrying a capability template): not migrated (§4.2).
pub fn upgrade_local(local: &Connection) -> WsResult<()> {
    local.execute_batch("DROP TABLE IF EXISTS grants;").map_err(db_err)?;
    let template_form: bool = local
        .query_row("SELECT 1 FROM pragma_table_info('share_entries') WHERE name = 'capabilities'", [], |_| Ok(true))
        .optional()
        .map_err(db_err)?
        .unwrap_or(false);
    if template_form {
        local.execute_batch("DROP TABLE share_entries;").map_err(db_err)?;
    }
    Ok(())
}

/// Storage 6 → 7: entities learn their creator — the writer of the Commit that created them, the owner when that
/// history is gone — and Commits get room for their attribution. One transaction with the version change.
pub fn upgrade_doc_7(doc: &Connection, owner: Option<&str>) -> WsResult<()> {
    doc.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;
    let r = (|| -> WsResult<()> {
        doc.execute_batch("ALTER TABLE entities ADD COLUMN created_by TEXT; ALTER TABLE commits ADD COLUMN attribution_json TEXT;").map_err(db_err)?;
        doc.execute(
            "UPDATE entities SET created_by = COALESCE((SELECT principal FROM commits WHERE commits.seq = entities.created_seq), ?1) WHERE created_seq > 0",
            [owner],
        )
        .map_err(db_err)?;
        crate::docdb::meta_set(doc, "storage_schema_version", "7")?;
        Ok(())
    })();
    match r {
        Ok(()) => doc.execute_batch("COMMIT").map_err(db_err),
        Err(e) => {
            let _ = doc.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// How one caller is judged on one Workspace (§4.6 step 3).
#[derive(Debug, Clone, Default)]
pub struct Judgment {
    /// Who it is judged as: itself, or — an Agent this Workspace configured nothing for — its owner (R16).
    pub principal: String,
    pub owner: bool,
    /// The logical groups it belongs to, each with the member specs that put it there.
    pub groups: BTreeMap<String, Vec<String>>,
    /// Some entry names it.
    pub direct: bool,
    /// In `@guest`: anonymous access is on and it belongs to no group.
    pub guest: bool,
    /// May enter the Workspace (and so is in `@all`).
    pub member: bool,
    pub agent_as_owner: bool,
    pub publication: Option<String>,
}

impl Judgment {
    /// The subjects this judgment holds (what hosted execution's `triggerers` are compared with).
    pub fn subjects(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::from([self.principal.clone()]);
        if self.owner {
            out.insert(SUBJECT_OWNER.into());
        }
        out.extend(self.groups.keys().map(|g| format!("@{g}")));
        if self.member && self.publication.is_none() {
            out.insert(SUBJECT_ALL.into());
        }
        if self.guest {
            out.insert(SUBJECT_GUEST.into());
        }
        out
    }

    pub fn to_json(&self, data: &AclData) -> Value {
        let groups: Vec<Value> = self
            .groups
            .iter()
            .map(|(g, via)| {
                let entries: Vec<Value> = via
                    .iter()
                    .map(|m| {
                        let row = data.members.iter().find(|r| &r.group_id == g && &r.member == m);
                        json!({ "member": m, "via_entry": row.and_then(|r| r.via_entry.clone()) })
                    })
                    .collect();
                json!({ "group_id": g, "title": data.groups.get(g).map(|x| x.title.clone()), "via": entries })
            })
            .collect();
        json!({ "judged_as": self.principal, "owner": self.owner, "agent_as_owner": self.agent_as_owner, "groups": groups,
                "direct": self.direct, "guest": self.guest, "member": self.member })
    }
}

/// Judge `caller` against `data`: an Agent nobody configured here is judged as its owner (R16, Q29).
pub fn judge(data: &AclData, caller: &Caller) -> WsResult<Judgment> {
    if caller.is_agent() && caller.publication.is_none() && !data.configures(&caller.principal) {
        if let Some(owner) = &caller.acting_for {
            let mut j = judge_one(data, owner)?;
            j.agent_as_owner = true;
            return Ok(j);
        }
    }
    judge_one(data, caller)
}

fn judge_one(data: &AclData, caller: &Caller) -> WsResult<Judgment> {
    if let Some(p) = &caller.publication {
        let direct = data.has_entries(&caller.principal);
        return Ok(Judgment { principal: caller.principal.clone(), direct, member: direct, publication: Some(p.clone()), ..Judgment::default() });
    }
    let owner = !caller.is_guest() && data.owner.as_deref() == Some(caller.principal.as_str());
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut undecided: BTreeSet<String> = BTreeSet::new();
    for m in &data.members {
        match caller.matches_member(&m.member) {
            MemberMatch::Yes => groups.entry(m.group_id.clone()).or_default().push(m.member.clone()),
            MemberMatch::Unknown => {
                undecided.insert(m.group_id.clone());
            }
            MemberMatch::No => {}
        }
    }
    undecided.retain(|g| !groups.contains_key(g));
    if !owner && !undecided.is_empty() {
        // one group less may turn a refusal into an allowance (§4.6): refuse when it could matter
        let matters = undecided.iter().any(|g| data.has_entries(&format!("@{g}"))) || (data.anonymous && groups.is_empty() && data.has_entries(SUBJECT_GUEST));
        if matters {
            return Err(WsError::refused(Code::PermissionDenied, Deny::AuthorizationUnavailable, "a group or contact membership could not be checked; retry later"));
        }
    }
    let direct = data.has_entries(&caller.principal);
    let guest = !owner && data.anonymous && groups.is_empty();
    let member = owner || !groups.is_empty() || direct || guest;
    Ok(Judgment { principal: caller.principal.clone(), owner, groups, direct, guest, member, agent_as_owner: false, publication: None })
}

/// Where an entry's subject applies to a judged caller: its tier, and whether it is `@creator`'s.
fn applies(j: &Judgment, subject: &str) -> Option<(Tier, bool)> {
    if subject == j.principal {
        return Some((Tier::Single, false));
    }
    if j.publication.is_some() {
        return None;
    }
    match subject {
        SUBJECT_ALL if j.member => Some((Tier::Everyone, false)),
        SUBJECT_GUEST if j.guest => Some((Tier::Everyone, false)),
        SUBJECT_CREATOR if j.member => Some((Tier::Group, true)),
        s => s.strip_prefix('@').filter(|g| j.groups.contains_key(*g)).map(|_| (Tier::Group, false)),
    }
}

pub fn access_of(data: &AclData, j: &Judgment) -> Access {
    if j.owner {
        return Access::full(&j.principal);
    }
    let mut layers: BTreeMap<String, Vec<Applied>> = BTreeMap::new();
    for e in &data.entries {
        if let Some((tier, creator)) = applies(j, &e.subject) {
            layers.entry(e.scope.clone()).or_default().push(Applied { subject: e.subject.clone(), tier, creator, allow: e.allow, deny: e.deny });
        }
    }
    Access::from_layers(&j.principal, j.member, layers)
}

/// The effective permissions of a logical group by itself: what a member of only that group holds (its entries and
/// `@all`'s). A macro bound with `intersection` never exceeds it (§4.5).
pub fn group_access(data: &AclData, group_id: &str) -> Access {
    let j = Judgment { principal: format!("@{group_id}"), groups: BTreeMap::from([(group_id.to_string(), vec![])]), member: true, ..Judgment::default() };
    access_of(data, &j)
}

/// Kind of a subject as the queries report it.
fn subject_kind(subject: &str) -> &'static str {
    match subject {
        SUBJECT_OWNER | SUBJECT_ALL | SUBJECT_GUEST | SUBJECT_CREATOR => "fixed",
        s if s.starts_with(PUB_PREFIX) => "publication",
        s if s.starts_with('@') => "group",
        _ => "single",
    }
}

fn subject_tier(subject: &str) -> Tier {
    match subject_kind(subject) {
        "single" | "publication" => Tier::Single,
        "group" => Tier::Group,
        _ if subject == SUBJECT_CREATOR => Tier::Group,
        _ => Tier::Everyone,
    }
}

pub fn is_group_id(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=64).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
        && !RESERVED_GROUP_IDS.contains(&s)
}

/// A single caller's principal as an entry or member names it: a user id, a DID, `guest:…`, `link:…`.
fn check_principal(s: &str) -> WsResult<()> {
    if s.is_empty() || s.len() > 256 || s.chars().any(char::is_whitespace) || s.starts_with('@') {
        return Err(WsError::invalid_op(format!("invalid principal {s:?}: 1–256 characters without spaces")));
    }
    if s == ANY_ACCOUNT_MEMBER || s.starts_with(GROUP_PREFIX) || s.starts_with(CONTACT_PREFIX) || s == "public" {
        return Err(WsError::invalid_op(format!("{s} is a member of a logical group, not a subject: add it to a group (acl.members) and name the group")));
    }
    Ok(())
}

fn caps_json(c: CapSet) -> Value {
    json!(c.names())
}

/// One recorded change.
pub struct AuditItem<'a> {
    pub action: &'a str,
    pub target: Value,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

impl Workspace {
    /// The permission data decisions read: a show clone judges by its source Workspace's, as they are now (D17).
    pub fn acl_data(&self) -> WsResult<AclData> {
        if let Some(source) = meta_get(&self.doc, "show_source")? {
            let Some(parent) = self.dir.parent() else { return Ok(AclData::default()) };
            let path = parent.join(&source).join("local.sqlite");
            if !path.exists() {
                return Ok(AclData::default());
            }
            let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(db_err)?;
            return AclData::load(&conn);
        }
        AclData::load(&self.local)
    }

    pub fn judge(&self, caller: &Caller) -> WsResult<Judgment> {
        judge(&self.acl_data()?, caller)
    }

    /// The caller's effective permissions here (a macro's caller: its macro identity, [`Self::macro_access`]).
    pub fn access(&self, caller: &Caller) -> WsResult<Access> {
        let data = self.acl_data()?;
        let base = access_of(&data, &judge(&data, caller)?);
        match caller.macro_of.as_deref() {
            Some(wish_id) => self.bounded_by_binding(&data, base, wish_id),
            None => Ok(base),
        }
    }

    pub fn subjects_of(&self, caller: &Caller) -> WsResult<BTreeSet<String>> {
        Ok(self.judge(caller)?.subjects())
    }

    /// What `ws.list` shows: the owner's Workspaces and those naming the caller or one of its groups — never one it
    /// could only enter anonymously (R8).
    pub fn listed(&self, caller: &Caller) -> WsResult<Option<Access>> {
        let data = self.acl_data()?;
        let j = judge(&data, caller)?;
        Ok((j.owner || !j.groups.is_empty() || j.direct).then(|| access_of(&data, &j)))
    }

    /// Member specs whose membership the service settles per request: `group:` and `contact:`.
    pub fn dynamic_subjects(&self) -> WsResult<Vec<String>> {
        let data = self.acl_data()?;
        let set: BTreeSet<String> = data.members.into_iter().map(|m| m.member).filter(|m| m.starts_with(GROUP_PREFIX) || m.starts_with(CONTACT_PREFIX)).collect();
        Ok(set.into_iter().collect())
    }

    /// May enter: anything at all here (otherwise the Workspace does not exist for the caller).
    pub fn require_ws_any(&self, caller: &Caller) -> WsResult<Access> {
        let access = self.access(caller)?;
        if access.is_empty() {
            return Err(WsError::not_found("workspace not found"));
        }
        Ok(access)
    }

    /// `cap` on the Workspace itself (`/`).
    pub fn require_ws(&self, caller: &Caller, cap: Cap) -> WsResult<Access> {
        let access = self.require_ws_any(caller)?;
        if !access.ws_caps.has(cap) {
            return Err(WsError::denied(format!("{} capability required", cap_name(cap))));
        }
        Ok(access)
    }

    /// `cap` on an entity (`None`: the Workspace).
    pub fn require_on(&self, caller: &Caller, entity_id: Option<&str>, cap: Cap) -> WsResult<Access> {
        let access = self.require_ws_any(caller)?;
        let ok = match entity_id {
            None => access.ws_caps.has(cap),
            Some(id) => {
                let now = self.now();
                access.can(&self.ctx(&now), id, cap)?
            }
        };
        if !ok {
            return Err(WsError::denied(format!("{} capability required", cap_name(cap))));
        }
        Ok(access)
    }

    /// The owner's own operations (R17).
    pub fn require_owner(&self, caller: &Caller) -> WsResult<Access> {
        let access = self.require_ws_any(caller)?;
        if !access.is_owner() {
            return Err(WsError::sub(Code::PermissionDenied, "OWNER_ONLY", "only the workspace's owner may do this"));
        }
        Ok(access)
    }

    /// May start a run of `wish_id` (§4.5, R22): `run` or any editing right on the wish.
    pub fn require_run(&self, caller: &Caller, wish_id: Option<&str>) -> WsResult<Access> {
        let access = self.require_ws_any(caller)?;
        let caps = match wish_id {
            Some(id) => {
                let now = self.now();
                access.caps(&self.ctx(&now), id)?
            }
            None => access.ws_caps,
        };
        if !caps.may_run() {
            return Err(WsError::denied("run capability required"));
        }
        Ok(access)
    }

    pub fn anonymous(&self) -> WsResult<bool> {
        Ok(self.local.query_row("SELECT value FROM local_meta WHERE key = 'anonymous'", [], |r| r.get::<_, String>(0)).optional().map_err(db_err)?.as_deref() == Some("1"))
    }

    /// Changes of permissions so far: the change stream reports it (R4).
    pub fn acl_rev(&self) -> u64 {
        self.local.query_row("SELECT value FROM local_meta WHERE key = 'acl_rev'", [], |r| r.get::<_, String>(0)).ok().and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    fn bump_acl_rev(&self, tx: &Connection) -> WsResult<u64> {
        let rev = self.acl_rev() + 1;
        tx.execute("INSERT OR REPLACE INTO local_meta (key, value) VALUES ('acl_rev', ?1)", [rev.to_string()]).map_err(db_err)?;
        Ok(rev)
    }

    fn acl_changed(&self, rev: u64) {
        if let Some(f) = &self.on_acl_change {
            f(rev);
        }
    }

    /// Write audit records and bump the permission revision, in one transaction with `write`; then wake the stream.
    pub(crate) fn acl_txn<T>(&self, caller: &Caller, write: impl FnOnce(&Connection) -> WsResult<(T, Vec<AuditItem<'_>>)>) -> WsResult<(T, u64)> {
        let judged = self.judge(caller).map(|j| j.principal).unwrap_or_else(|_| caller.principal.clone());
        let tx = self.local.unchecked_transaction().map_err(db_err)?;
        let (out, items) = write(&tx)?;
        let now = self.now();
        for it in &items {
            tx.execute(
                "INSERT INTO acl_audit (at, actor, judged_as, action, target_json, before_json, after_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![now, caller.principal, (judged != caller.principal).then_some(&judged), it.action, it.target.to_string(),
                        it.before.as_ref().map(Value::to_string), it.after.as_ref().map(Value::to_string)],
            )
            .map_err(db_err)?;
        }
        let rev = self.bump_acl_rev(&tx)?;
        tx.commit().map_err(db_err)?;
        self.acl_changed(rev);
        Ok((out, rev))
    }

    /// One operation record outside a permission change (publications, hosted execution, moves widening visibility).
    pub(crate) fn audit(&self, caller: &Caller, action: &str, target: &Value, before: Option<&Value>, after: Option<&Value>) -> WsResult<()> {
        let judged = self.judge(caller).map(|j| j.principal).unwrap_or_else(|_| caller.principal.clone());
        self.local
            .execute(
                "INSERT INTO acl_audit (at, actor, judged_as, action, target_json, before_json, after_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![self.now(), caller.principal, (judged != caller.principal).then_some(&judged), action, target.to_string(),
                        before.map(Value::to_string), after.map(Value::to_string)],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// A resource as interfaces name it — a path (`/`, `/data/销售`, `~<id>/…`) or an entity id — to the scope it is
    /// stored under (`""` = the Workspace). Each step must be readable and live (§4.1); personal and session scoped
    /// entities are not targets.
    pub fn resolve_resource(&self, access: &Access, resource: &str) -> WsResult<String> {
        let r = resource.trim();
        if r.is_empty() || r == "/" || r == ROOT_ID {
            return Ok(String::new());
        }
        let now = self.now();
        let ctx = self.ctx(&now);
        let id = aiworkspace_core::path::resolve(&ctx, access, r)?;
        if id == ROOT_ID {
            return Ok(String::new());
        }
        let e = ctx.entity(&id)?.ok_or_else(|| WsError::not_found(format!("{r} not found")))?;
        if e.scope != SCOPE_SHARED {
            return Err(WsError::invalid_op("personal and session scoped entities are not permission targets: their scope decides who sees them"));
        }
        Ok(id)
    }

    /// `{ entity_id, path }` of a scope as `access` may print it.
    pub fn resource_json(&self, access: &Access, scope: &str) -> Value {
        if scope.is_empty() {
            return json!({ "entity_id": Value::Null, "path": "/" });
        }
        let now = self.now();
        let ctx = self.ctx(&now);
        let path = aiworkspace_core::path::path_of(&ctx, access, scope).unwrap_or_else(|_| format!("~{scope}"));
        json!({ "entity_id": scope, "path": path })
    }

    /// Groups that may never carry `manage` (entry targets, macro-bound groups) or `run` (entry targets): R18.
    fn group_limits(&self, group_id: &str) -> WsResult<(bool, bool)> {
        let entry_target: bool = self
            .local
            .query_row("SELECT 1 FROM share_entries WHERE group_id = ?1 AND state = 'active' LIMIT 1", [group_id], |_| Ok(true))
            .optional()
            .map_err(db_err)?
            .unwrap_or(false);
        let bound: bool = self.local.query_row("SELECT 1 FROM acl_macro_bindings WHERE group_id = ?1 LIMIT 1", [group_id], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
        Ok((entry_target || bound, entry_target))
    }

    /// Is `subject` acceptable on an entry allowing `allow` (§4.2, R18)?
    fn check_entry(&self, data: &AclData, subject: &str, allow: CapSet) -> WsResult<()> {
        let refuse = |what: &str| Err(WsError::invalid_op(format!("{subject} cannot be allowed {what} (权限需求 R18)")));
        match subject {
            SUBJECT_OWNER => return Err(WsError::invalid_op("@owner always holds everything: it takes no entry")),
            SUBJECT_ALL | SUBJECT_GUEST => {
                if allow.has(Cap::Manage) || allow.has(Cap::Run) {
                    return refuse("manage or run");
                }
            }
            SUBJECT_CREATOR => {
                if allow.has(Cap::Manage) {
                    return refuse("manage");
                }
            }
            // a publication's own entry is written by ws.publish; acl.set may only refuse it something
            s if s.starts_with(PUB_PREFIX) => {
                if !allow.is_empty() {
                    return Err(WsError::invalid_op("a publication's subject is allowed by ws.publish only; acl.set may deny it"));
                }
            }
            s if s.starts_with('@') => {
                let g = &s[1..];
                if !data.groups.contains_key(g) {
                    return Err(WsError::not_found(format!("group {g} not found")));
                }
                let (no_manage, no_run) = self.group_limits(g)?;
                if no_manage && allow.has(Cap::Manage) {
                    return refuse("manage: it is a share entry's or a macro's group");
                }
                if no_run && allow.has(Cap::Run) {
                    return refuse("run: it is a share entry's group");
                }
            }
            s => check_principal(s)?,
        }
        Ok(())
    }

    /// `acl.set { changes: [{ subject, resource, allow, deny }] }` (R2): each change replaces the subject's entry on
    /// the resource (both empty: removed), all or none, one operation record. `manage` on each resource; anything
    /// touching `manage` is the owner's (R17).
    pub fn acl_set(&mut self, caller: &Caller, changes: &Value) -> WsResult<Value> {
        let list = changes.as_array().ok_or_else(|| WsError::invalid_op("changes must be an array"))?;
        if list.is_empty() || list.len() > MAX_CHANGES {
            return Err(WsError::invalid_op(format!("changes: 1…{MAX_CHANGES} items")));
        }
        let access = self.require_ws_any(caller)?;
        let data = self.acl_data()?;
        let now = self.now();
        let mut plan: Vec<(String, String, CapSet, CapSet, Option<EntryRow>)> = Vec::new();
        for (i, c) in list.iter().enumerate() {
            let at = |e: WsError| e.at(format!("/changes/{i}"));
            let subject = c.get("subject").and_then(Value::as_str).ok_or_else(|| at(WsError::invalid_op("subject required")))?.to_string();
            let resource = c.get("resource").and_then(Value::as_str).unwrap_or("/");
            let allow = CapSet::parse_json(c.get("allow"), "allow").map_err(at)?;
            let deny = CapSet::parse_json(c.get("deny"), "deny").map_err(at)?;
            let scope = match self.resolve_resource(&access, resource) {
                Ok(s) => s,
                // an entry left on what was deleted can still be removed by its id
                Err(e) => {
                    let raw = resource.trim_start_matches('~');
                    if allow.is_empty() && deny.is_empty() && data.entries.iter().any(|x| x.scope == raw && x.subject == subject) {
                        raw.to_string()
                    } else {
                        return Err(at(e));
                    }
                }
            };
            let manages = if scope.is_empty() { access.ws_caps.has(Cap::Manage) } else { access.can(&self.ctx(&now), &scope, Cap::Manage)? };
            if !manages {
                return Err(at(WsError::denied("manage capability required on the resource")));
            }
            if !(allow.is_empty() && deny.is_empty()) {
                self.check_entry(&data, &subject, allow).map_err(at)?;
            }
            let old = data.entries.iter().find(|x| x.scope == scope && x.subject == subject).cloned();
            let touches_manage = allow.union(deny).has(Cap::Manage) || old.as_ref().is_some_and(|o| o.allow.union(o.deny).has(Cap::Manage));
            if touches_manage && !access.is_owner() {
                return Err(at(WsError::sub(Code::PermissionDenied, "OWNER_ONLY", "only the owner gives or takes the manage capability (权限需求 R17)")));
            }
            plan.push((subject, scope, allow, deny, old));
        }
        let entry_json = |subject: &str, scope: &str, allow: CapSet, deny: CapSet| json!({ "subject": subject, "resource": self.resource_json(&access, scope), "allow": caps_json(allow), "deny": caps_json(deny) });
        let (out, rev) = self.acl_txn(caller, |tx| {
            let mut befores = Vec::new();
            let mut afters = Vec::new();
            for (subject, scope, allow, deny, old) in &plan {
                if allow.is_empty() && deny.is_empty() {
                    tx.execute("DELETE FROM acl_entries WHERE subject = ?1 AND scope_entity_id = ?2", params![subject, scope]).map_err(db_err)?;
                } else {
                    tx.execute(
                        "INSERT OR REPLACE INTO acl_entries (subject, scope_entity_id, allow, deny, updated_by, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![subject, scope, allow.0, deny.0, caller.principal, now],
                    )
                    .map_err(db_err)?;
                }
                befores.push(old.as_ref().map(|o| entry_json(subject, scope, o.allow, o.deny)).unwrap_or(Value::Null));
                afters.push(entry_json(subject, scope, *allow, *deny));
            }
            let item = AuditItem { action: "acl.set", target: json!(plan.iter().map(|(s, sc, ..)| json!({ "subject": s, "resource": self.resource_json(&access, sc) })).collect::<Vec<_>>()),
                                   before: Some(json!(befores)), after: Some(json!(afters.clone())) };
            Ok((afters, vec![item]))
        })?;
        Ok(json!({ "ok": true, "acl_rev": rev, "changes": out }))
    }

    /// `acl.set_anonymous { enabled }` (`manage`): whether `@guest` exists here at all (§4.2, Q22).
    pub fn acl_set_anonymous(&mut self, caller: &Caller, enabled: bool) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let before = self.anonymous()?;
        let (_, rev) = self.acl_txn(caller, |tx| {
            tx.execute("INSERT OR REPLACE INTO local_meta (key, value) VALUES ('anonymous', ?1)", [if enabled { "1" } else { "0" }]).map_err(db_err)?;
            Ok(((), vec![AuditItem { action: "acl.set_anonymous", target: json!({}), before: Some(json!(before)), after: Some(json!(enabled)) }]))
        })?;
        Ok(json!({ "ok": true, "anonymous": enabled, "acl_rev": rev }))
    }

    /// `acl.set_group { group_id, title?, description? }` (`manage`): creates the group, or changes its title and
    /// description (never its identity).
    pub fn acl_set_group(&mut self, caller: &Caller, group_id: &str, title: Option<&str>, description: Option<&str>) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        if !is_group_id(group_id) {
            return Err(WsError::invalid_op("group_id: [a-z0-9][a-z0-9_-]{0,63}, not owner, all, guest, creator or pub"));
        }
        for (k, v) in [("title", title), ("description", description)] {
            if v.is_some_and(|v| v.chars().count() > if k == "title" { 128 } else { 2000 }) {
                return Err(WsError::invalid_op(format!("{k} too long")));
            }
        }
        let data = self.acl_data()?;
        let old = data.groups.get(group_id).cloned();
        if old.is_none() && data.groups.len() >= MAX_GROUPS {
            return Err(WsError::limit(format!("at most {MAX_GROUPS} groups")));
        }
        let now = self.now();
        let title = title.map(str::to_string).or_else(|| old.as_ref().map(|g| g.title.clone())).unwrap_or_else(|| group_id.to_string());
        let description = description.map(str::to_string).or_else(|| old.as_ref().map(|g| g.description.clone())).unwrap_or_default();
        let after = json!({ "group_id": group_id, "title": title, "description": description });
        let before = old.as_ref().map(|g| json!({ "group_id": g.group_id, "title": g.title, "description": g.description }));
        let (_, rev) = self.acl_txn(caller, |tx| {
            match &old {
                Some(_) => tx.execute("UPDATE acl_groups SET title = ?2, description = ?3 WHERE group_id = ?1", params![group_id, title, description]),
                None => tx.execute("INSERT INTO acl_groups (group_id, title, description, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![group_id, title, description, caller.principal, now]),
            }
            .map_err(db_err)?;
            Ok(((), vec![AuditItem { action: if old.is_some() { "acl.update_group" } else { "acl.create_group" }, target: json!({ "group_id": group_id }), before: before.clone(), after: Some(after.clone()) }]))
        })?;
        Ok(json!({ "ok": true, "group": after, "created": old.is_none(), "acl_rev": rev }))
    }

    /// `acl.delete_group { group_id }` (`manage`; the owner's when the group holds `manage`): its entries, members,
    /// macro bindings go; entries leading into it close.
    pub fn acl_delete_group(&mut self, caller: &Caller, group_id: &str) -> WsResult<Value> {
        let access = self.require_ws(caller, Cap::Manage)?;
        let data = self.acl_data()?;
        let g = data.groups.get(group_id).cloned().ok_or_else(|| WsError::not_found(format!("group {group_id} not found")))?;
        let subject = format!("@{group_id}");
        if data.entries.iter().any(|e| e.subject == subject && e.allow.union(e.deny).has(Cap::Manage)) && !access.is_owner() {
            return Err(WsError::sub(Code::PermissionDenied, "OWNER_ONLY", "the group holds the manage capability: only the owner deletes it"));
        }
        let entries: Vec<Value> = data.entries.iter().filter(|e| e.subject == subject).map(|e| json!({ "resource": self.resource_json(&access, &e.scope), "allow": caps_json(e.allow), "deny": caps_json(e.deny) })).collect();
        let members: Vec<&str> = data.members.iter().filter(|m| m.group_id == group_id).map(|m| m.member.as_str()).collect();
        let before = json!({ "group_id": g.group_id, "title": g.title, "description": g.description, "entries": entries, "members": members });
        let (_, rev) = self.acl_txn(caller, |tx| {
            tx.execute("DELETE FROM acl_entries WHERE subject = ?1", [&subject]).map_err(db_err)?;
            tx.execute("DELETE FROM acl_members WHERE group_id = ?1", [group_id]).map_err(db_err)?;
            tx.execute("DELETE FROM acl_macro_bindings WHERE group_id = ?1", [group_id]).map_err(db_err)?;
            tx.execute("UPDATE share_entries SET state = 'closed' WHERE group_id = ?1", [group_id]).map_err(db_err)?;
            tx.execute("DELETE FROM acl_groups WHERE group_id = ?1", [group_id]).map_err(db_err)?;
            Ok(((), vec![AuditItem { action: "acl.delete_group", target: json!({ "group_id": group_id }), before: Some(before.clone()), after: None }]))
        })?;
        Ok(json!({ "ok": true, "group_id": group_id, "acl_rev": rev }))
    }

    /// A member spec as a group takes it (§4.2): a principal, `group:<id>`, `contact:<your did>#<tag>`, `*`. A group
    /// is never a member of another group.
    fn check_member(&self, caller: &Caller, member: &str) -> WsResult<()> {
        if member.starts_with('@') {
            return Err(WsError::invalid_op("a group cannot be a member of another group"));
        }
        if member == ANY_ACCOUNT_MEMBER {
            return Ok(());
        }
        if let Some(g) = member.strip_prefix(GROUP_PREFIX) {
            return if g.is_empty() || g.len() > 128 { Err(WsError::invalid_op("group:<id>")) } else { Ok(()) };
        }
        if let Some(contact) = member.strip_prefix(CONTACT_PREFIX) {
            return match contact.split_once('#') {
                Some((owner, tag)) if owner.starts_with("did:") && !tag.is_empty() => {
                    if caller.did() != Some(owner) {
                        return Err(WsError::invalid_op("a contact group is named by its owner only (contact:<your did>#<tag>)"));
                    }
                    Ok(())
                }
                _ => Err(WsError::invalid_op("contact:<owner_did>#<tag>")),
            };
        }
        if member.len() > 256 || member.is_empty() || member.chars().any(char::is_whitespace) || member == "public" {
            return Err(WsError::invalid_op(format!("invalid member {member:?}")));
        }
        Ok(())
    }

    /// `acl.members { group_id, add?, remove? }` (`manage`; the owner's when the group holds `manage`, R17).
    pub fn acl_members(&mut self, caller: &Caller, group_id: &str, add: &[String], remove: &[String]) -> WsResult<Value> {
        let access = self.require_ws(caller, Cap::Manage)?;
        let data = self.acl_data()?;
        if !data.groups.contains_key(group_id) {
            return Err(WsError::not_found(format!("group {group_id} not found")));
        }
        if add.len() + remove.len() > MAX_CHANGES || add.len() + remove.len() == 0 {
            return Err(WsError::invalid_op(format!("add and remove: 1…{MAX_CHANGES} members")));
        }
        let subject = format!("@{group_id}");
        if data.entries.iter().any(|e| e.subject == subject && e.allow.has(Cap::Manage)) && !access.is_owner() {
            return Err(WsError::sub(Code::PermissionDenied, "OWNER_ONLY", "the group holds the manage capability: only the owner changes its members (权限需求 R17)"));
        }
        for m in add {
            self.check_member(caller, m)?;
        }
        if data.members.iter().filter(|m| m.group_id == group_id).count() + add.len() > MAX_MEMBERS {
            return Err(WsError::limit(format!("a group has at most {MAX_MEMBERS} members")));
        }
        let now = self.now();
        let (n, rev) = self.acl_txn(caller, |tx| {
            let (mut added, mut removed) = (Vec::new(), Vec::new());
            for m in add {
                if tx.execute("INSERT OR IGNORE INTO acl_members (group_id, member, via_entry, added_by, added_at) VALUES (?1, ?2, NULL, ?3, ?4)", params![group_id, m, caller.principal, now]).map_err(db_err)? > 0 {
                    added.push(m.clone());
                }
            }
            for m in remove {
                if tx.execute("DELETE FROM acl_members WHERE group_id = ?1 AND member = ?2", params![group_id, m]).map_err(db_err)? > 0 {
                    removed.push(m.clone());
                }
            }
            let item = AuditItem { action: "acl.members", target: json!({ "group_id": group_id }), before: Some(json!({ "removed": removed })), after: Some(json!({ "added": added })) };
            Ok(((added.len(), removed.len()), vec![item]))
        })?;
        Ok(json!({ "ok": true, "group_id": group_id, "added": n.0, "removed": n.1, "acl_rev": rev }))
    }

    /// `acl.groups`: every group with its members for a manager; anyone else sees the groups it belongs to (Q6).
    pub fn acl_groups(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let data = self.acl_data()?;
        let manage = access.ws_caps.has(Cap::Manage);
        let mine = judge(&data, caller)?;
        let groups: Vec<Value> = data
            .groups
            .values()
            .filter(|g| manage || mine.groups.contains_key(&g.group_id))
            .map(|g| {
                let mut v = json!({ "group_id": g.group_id, "subject": format!("@{}", g.group_id), "title": g.title, "description": g.description,
                                     "created_by": g.created_by, "created_at": g.created_at, "member": mine.groups.contains_key(&g.group_id) });
                if manage {
                    v["members"] = json!(data.members.iter().filter(|m| m.group_id == g.group_id)
                        .map(|m| json!({ "member": m.member, "via_entry": m.via_entry.as_ref().map(|e| self.entry_public_id(e)), "added_by": m.added_by, "added_at": m.added_at }))
                        .collect::<Vec<_>>());
                }
                v
            })
            .collect();
        Ok(json!({ "ok": true, "groups": groups, "complete": manage, "anonymous": data.anonymous }))
    }

    /// `acl.bind_macro { wish, group_id | null, mode }` (`manage`): the macro of a wish runs with the triggerer's
    /// permissions ∪ or ∩ the group's (§4.5). Bound by wish id, not version; whoever may change the wish holds the
    /// group's permissions through it (Q10), so the group may never hold `manage`.
    pub fn acl_bind_macro(&mut self, caller: &Caller, wish: &str, group_id: Option<&str>, mode: &str) -> WsResult<Value> {
        let access = self.require_ws(caller, Cap::Manage)?;
        let wish_id = self.resolve_resource(&access, wish)?;
        let now = self.now();
        if !self.ctx(&now).entity(&wish_id)?.is_some_and(|e| e.type_id == TYPE_WISH && e.alive()) {
            return Err(WsError::invalid_op(format!("{wish} is not a wish")));
        }
        if !matches!(mode, "union" | "intersection") {
            return Err(WsError::invalid_op("mode must be union or intersection"));
        }
        let data = self.acl_data()?;
        if let Some(g) = group_id {
            if !data.groups.contains_key(g) {
                return Err(WsError::not_found(format!("group {g} not found")));
            }
            if data.allowed_anywhere(&format!("@{g}")).has(Cap::Manage) {
                return Err(WsError::invalid_op("a macro's group cannot hold the manage capability"));
            }
        }
        let before = self.macro_binding(&wish_id)?.map(|(g, m)| json!({ "group_id": g, "mode": m }));
        let after = group_id.map(|g| json!({ "group_id": g, "mode": mode }));
        let target = json!({ "wish": self.resource_json(&access, &wish_id) });
        let (_, rev) = self.acl_txn(caller, |tx| {
            match group_id {
                Some(g) => tx.execute("INSERT OR REPLACE INTO acl_macro_bindings (wish_id, group_id, mode, set_by, set_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![wish_id, g, mode, caller.principal, now]),
                None => tx.execute("DELETE FROM acl_macro_bindings WHERE wish_id = ?1", [&wish_id]),
            }
            .map_err(db_err)?;
            Ok(((), vec![AuditItem { action: "acl.bind_macro", target: target.clone(), before: before.clone(), after: after.clone() }]))
        })?;
        Ok(json!({ "ok": true, "wish_id": wish_id, "binding": after, "acl_rev": rev,
                   "warning": "whoever may change this wish's macro holds the group's permissions through it" }))
    }

    /// The group a wish's macro is bound to and how: `(group_id, union | intersection)`.
    pub fn macro_binding(&self, wish_id: &str) -> WsResult<Option<(String, String)>> {
        self.local.query_row("SELECT group_id, mode FROM acl_macro_bindings WHERE wish_id = ?1", [wish_id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(db_err)
    }

    /// The macro identity of a run of `wish_id` by `triggerer` (§4.5): the triggerer's access, bounded by the bound
    /// group with `intersection`. `union` adds nothing in this phase: only a trusted executor may run with more than
    /// the triggerer holds (Q12), and none is supported yet.
    pub fn macro_access(&self, triggerer: &Caller, wish_id: &str) -> WsResult<Access> {
        let data = self.acl_data()?;
        let base = access_of(&data, &judge(&data, triggerer)?);
        self.bounded_by_binding(&data, base, wish_id)
    }

    fn bounded_by_binding(&self, data: &AclData, base: Access, wish_id: &str) -> WsResult<Access> {
        match self.macro_binding(wish_id)? {
            Some((g, mode)) if mode == "intersection" && data.groups.contains_key(&g) => Ok(base.bounded_by(group_access(data, &g))),
            _ => Ok(base),
        }
    }

    /// `acl.audit { after?, limit? }` (`manage`): the operation record, oldest first.
    pub fn acl_audit(&self, caller: &Caller, after: i64, limit: usize) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let mut st = self
            .local
            .prepare("SELECT seq, at, actor, judged_as, action, target_json, before_json, after_json FROM acl_audit WHERE seq > ?1 ORDER BY seq LIMIT ?2")
            .map_err(db_err)?;
        let parse = |s: Option<String>| s.and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(Value::Null);
        let items: Vec<Value> = st
            .query_map(params![after, limit.clamp(1, 1000) as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, Option<String>>(7)?))
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?
            .into_iter()
            .map(|(seq, at, actor, judged, action, target, before, after)| {
                json!({ "seq": seq, "at": at, "actor": actor, "judged_as": judged, "action": action, "target": parse(Some(target)), "before": parse(before), "after": parse(after) })
            })
            .collect();
        Ok(json!({ "ok": true, "items": items }))
    }

    /// The entries of one subject alone along a resource's chain decide what it holds there (by-entity queries).
    fn subject_effective(&self, data: &AclData, subject: &str, scope: &str) -> WsResult<CapSet> {
        let mut layers: BTreeMap<String, Vec<Applied>> = BTreeMap::new();
        for e in data.entries.iter().filter(|e| e.subject == subject) {
            layers.entry(e.scope.clone()).or_default().push(Applied { subject: subject.to_string(), tier: subject_tier(subject), creator: false, allow: e.allow, deny: e.deny });
        }
        let a = Access::from_layers(subject, true, layers);
        if scope.is_empty() {
            return Ok(a.ws_caps);
        }
        let now = self.now();
        a.caps(&self.ctx(&now), scope)
    }

    /// The layers from `scope` up to the Workspace (nearest first).
    fn chain_of(&self, scope: &str) -> WsResult<Vec<String>> {
        let mut out = Vec::new();
        if !scope.is_empty() {
            let now = self.now();
            let ctx = self.ctx(&now);
            let mut cur = scope.to_string();
            for _ in 0..4096 {
                if cur != ROOT_ID {
                    out.push(cur.clone());
                }
                match ctx.edge(&cur)? {
                    Some(e) => cur = e.parent_id,
                    None => break,
                }
            }
        }
        out.push(String::new());
        Ok(out)
    }

    /// `acl.query { by: entity | subject | explain, … }` (§4.4, R3). `principal`: for `subject` and `explain` about
    /// someone else, that caller with its memberships settled by the service (managers only).
    pub fn acl_query(&self, caller: &Caller, p: &Value, other: Option<&Caller>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let data = self.acl_data()?;
        let manage = access.ws_caps.has(Cap::Manage);
        match p.get("by").and_then(Value::as_str).unwrap_or("entity") {
            "entity" => {
                let scope = self.resolve_resource(&access, p.get("resource").and_then(Value::as_str).unwrap_or("/"))?;
                let manages = manage || (!scope.is_empty() && access.can(&self.ctx(&self.now()), &scope, Cap::Manage)?);
                if !manages {
                    return Err(WsError::denied("manage capability required: anyone else explains only themselves (acl.query by explain)"));
                }
                self.query_entity(&access, &data, &scope, p.get("expand").and_then(Value::as_bool).unwrap_or(false))
            }
            "subject" => {
                let subject = p.get("subject").and_then(Value::as_str).unwrap_or(&caller.principal).to_string();
                let me = subject == caller.principal;
                if !manage && !me {
                    return Err(WsError::denied("manage capability required to look at someone else's permissions"));
                }
                let mut v = json!({ "ok": true, "by": "subject", "subject": subject, "kind": subject_kind(&subject) });
                v["entries"] = json!(data.entries.iter().filter(|e| e.subject == subject)
                    .map(|e| json!({ "resource": self.resource_json(&access, &e.scope), "allow": caps_json(e.allow), "deny": caps_json(e.deny), "updated_by": e.updated_by, "updated_at": e.updated_at }))
                    .collect::<Vec<_>>());
                if subject_kind(&subject) == "group" {
                    let g = &subject[1..];
                    if !data.groups.contains_key(g) {
                        return Err(WsError::not_found(format!("group {g} not found")));
                    }
                    v["group"] = json!({ "group_id": g, "title": data.groups[g].title, "description": data.groups[g].description });
                    v["members"] = json!(data.members.iter().filter(|m| m.group_id == g).map(|m| json!({ "member": m.member, "via_entry": m.via_entry.as_ref().map(|e| self.entry_public_id(e)) })).collect::<Vec<_>>());
                } else if subject_kind(&subject) == "single" {
                    let who = if me { caller } else { other.ok_or_else(|| WsError::invalid_op("the service could not settle that principal's memberships"))? };
                    let j = judge(&data, who)?;
                    v["judgment"] = j.to_json(&data);
                    if j.agent_as_owner {
                        v["entries"] = json!([]);
                    }
                }
                Ok(v)
            }
            "explain" => {
                let me = other.is_none_or(|o| o.principal == caller.principal);
                if !me && !manage {
                    return Err(WsError::denied("manage capability required to explain someone else's permissions"));
                }
                let who = if me { caller } else { other.unwrap() };
                let scope = self.resolve_resource(&access, p.get("resource").and_then(Value::as_str).unwrap_or("/"))?;
                let j = judge(&data, who)?;
                let theirs = access_of(&data, &j);
                let now = self.now();
                let ctx = self.ctx(&now);
                let decisions = theirs.explain(&ctx, (!scope.is_empty()).then_some(scope.as_str()))?;
                let path_of = |id: &str| json!(aiworkspace_core::path::path_of(&ctx, &access, id).unwrap_or_else(|_| format!("~{id}")));
                let caps: Vec<Value> = decisions.iter().map(|d| d.to_json(&path_of)).collect();
                let mut v = json!({ "ok": true, "by": "explain", "principal": who.principal, "resource": self.resource_json(&access, &scope),
                                    "capabilities": caps, "effective": caps_json(CapSet(decisions.iter().filter(|d| d.allowed).fold(0, |b, d| b | d.cap as u16))) });
                v["judgment"] = j.to_json(&data);
                Ok(v)
            }
            // every entry (a manager), or the caller's own: what a client lists before it edits
            "all" => {
                // a manager sees every entry; anyone else the entries that apply to it — its own, its groups', `@all`, and
                // `@guest` while it is a guest (Q6: one's own groups, never their members)
                let mine = if manage { BTreeSet::new() } else { judge(&data, caller)?.subjects() };
                let entries: Vec<Value> = data
                    .entries
                    .iter()
                    .filter(|e| manage || mine.contains(&e.subject))
                    .map(|e| {
                        let mut v = json!({ "subject": e.subject, "kind": subject_kind(&e.subject), "resource": self.resource_json(&access, &e.scope),
                                            "allow": caps_json(e.allow), "deny": caps_json(e.deny) });
                        // who wrote an entry is the managers' to know (the operation record is theirs too)
                        if manage {
                            v["updated_by"] = json!(e.updated_by);
                            v["updated_at"] = json!(e.updated_at);
                        }
                        v
                    })
                    .collect();
                Ok(json!({ "ok": true, "by": "all", "entries": entries, "complete": manage, "owner": if manage { json!(data.owner) } else { Value::Null },
                           "principal": caller.principal, "anonymous": data.anonymous }))
            }
            other => Err(WsError::invalid_op(format!("by must be entity, subject, explain or all, not {other}"))),
        }
    }

    fn query_entity(&self, access: &Access, data: &AclData, scope: &str, expand: bool) -> WsResult<Value> {
        let chain = self.chain_of(scope)?;
        let source = |s: &str| if s == scope { "direct" } else if s.is_empty() { "workspace" } else { "inherited" };
        let on_chain: Vec<&EntryRow> = chain.iter().flat_map(|s| data.entries.iter().filter(move |e| &e.scope == s)).collect();
        let mut subjects: Vec<String> = Vec::new();
        for e in &on_chain {
            if !subjects.contains(&e.subject) {
                subjects.push(e.subject.clone());
            }
        }
        let rank = |s: &str| chain.iter().position(|x| x == s).unwrap_or(usize::MAX);
        let mut out = vec![json!({ "subject": SUBJECT_OWNER, "kind": "fixed", "principal": data.owner, "effective": caps_json(CapSet::ALL), "entries": [] })];
        for subject in &subjects {
            let entries: Vec<Value> = on_chain
                .iter()
                .filter(|e| &e.subject == subject)
                .map(|e| {
                    let mut v = json!({ "resource": self.resource_json(access, &e.scope), "source": source(&e.scope), "allow": caps_json(e.allow), "deny": caps_json(e.deny) });
                    if !e.deny.is_empty() {
                        // the allowances this refusal overrides for whoever holds both (§4.6: nearer layer, or same layer and a
                        // tier no more specific)
                        let tier = subject_tier(&e.subject);
                        let over: Vec<Value> = on_chain
                            .iter()
                            .filter(|a| a.subject != e.subject && !a.allow.intersect(e.deny).is_empty())
                            .filter(|a| rank(&a.scope) > rank(&e.scope) || (a.scope == e.scope && subject_tier(&a.subject) >= tier))
                            .map(|a| json!({ "subject": a.subject, "resource": self.resource_json(access, &a.scope), "capabilities": caps_json(a.allow.intersect(e.deny)) }))
                            .collect();
                        v["overrides"] = json!(over);
                    }
                    v
                })
                .collect();
            let mut item = json!({ "subject": subject, "kind": subject_kind(subject), "effective": caps_json(self.subject_effective(data, subject, scope)?), "entries": entries });
            if let Some(g) = subject.strip_prefix('@').and_then(|g| data.groups.get(g)) {
                item["title"] = json!(g.title);
                if expand {
                    item["members"] = json!(data.members.iter().filter(|m| m.group_id == g.group_id).map(|m| json!({ "member": m.member, "via_entry": m.via_entry.as_ref().map(|e| self.entry_public_id(e)) })).collect::<Vec<_>>());
                }
            }
            out.push(item);
        }
        // macros bound to a group with entries here: who may change the macro holds the group's permissions (Q10)
        let mut macros = Vec::new();
        let mut st = self.local.prepare_cached("SELECT wish_id, group_id, mode FROM acl_macro_bindings ORDER BY wish_id").map_err(db_err)?;
        let bindings: Vec<(String, String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        for (wish, g, mode) in bindings {
            let subject = format!("@{g}");
            if !subjects.contains(&subject) {
                continue;
            }
            macros.push(json!({ "wish": self.resource_json(access, &wish), "group_id": g, "mode": mode,
                                "capabilities": caps_json(self.subject_effective(data, &subject, scope)?), "editors": self.editors_of(data, &wish)? }));
        }
        Ok(json!({ "ok": true, "by": "entity", "resource": self.resource_json(access, scope), "subjects": out, "macros": macros, "anonymous": data.anonymous }))
    }

    /// The subjects that may change an entity's content (`update`) by their own entries, and the owner.
    fn editors_of(&self, data: &AclData, entity_id: &str) -> WsResult<Vec<String>> {
        let mut out = vec![SUBJECT_OWNER.to_string()];
        let subjects: BTreeSet<&str> = data.entries.iter().map(|e| e.subject.as_str()).collect();
        for s in subjects {
            if self.subject_effective(data, s, entity_id)?.has(Cap::Update) {
                out.push(s.to_string());
            }
        }
        Ok(out)
    }

    /// Who reaches `entity_id` with more after it moves from under `old_parent` to under `new_parent` (R23): each subject
    /// with entries here, judged by its own entries along both chains; the capabilities it gains.
    pub fn widened_by_move(&self, ctx: &dyn ReadCtx, entity_id: &str, old_parent: &str, new_parent: &str) -> WsResult<Vec<(String, CapSet)>> {
        let data = self.acl_data()?;
        if data.entries.iter().all(|e| e.scope.is_empty()) {
            return Ok(Vec::new());
        }
        let chain_from = |parent: &str| -> WsResult<Vec<String>> {
            let mut out = vec![entity_id.to_string()];
            let mut cur = parent.to_string();
            for _ in 0..4096 {
                if cur != ROOT_ID {
                    out.push(cur.clone());
                }
                match ctx.edge(&cur)? {
                    Some(e) => cur = e.parent_id,
                    None => break,
                }
            }
            out.push(String::new());
            Ok(out)
        };
        let (before, after) = (chain_from(old_parent)?, chain_from(new_parent)?);
        let subjects: BTreeSet<&str> = data.entries.iter().map(|e| e.subject.as_str()).collect();
        let mut out = Vec::new();
        for s in subjects {
            let mut layers: BTreeMap<String, Vec<Applied>> = BTreeMap::new();
            for e in data.entries.iter().filter(|e| e.subject == s) {
                layers.entry(e.scope.clone()).or_default().push(Applied { subject: s.to_string(), tier: subject_tier(s), creator: false, allow: e.allow, deny: e.deny });
            }
            let a = Access::from_layers(s, true, layers);
            let gained = a.caps_along(&after).minus(a.caps_along(&before));
            if !gained.is_empty() {
                out.push((s.to_string(), gained));
            }
        }
        Ok(out)
    }

    /// Entries on entities purged for good go with them (R20); a plain deletion keeps them for a restore.
    pub fn drop_entries_on(&self, entity_ids: &[String]) -> WsResult<u64> {
        let mut n = 0;
        for id in entity_ids {
            n += self.local.execute("DELETE FROM acl_entries WHERE scope_entity_id = ?1", [id]).map_err(db_err)? as u64;
            n += self.local.execute("DELETE FROM acl_macro_bindings WHERE wish_id = ?1", [id]).map_err(db_err)? as u64;
        }
        Ok(n)
    }

    /// The permission design (§4.3): group definitions, the entries of groups and fixed groups, macro bindings —
    /// what templates, clones and new imports carry; never members, single callers' entries, the anonymous switch
    /// or entries. `keep`: the entities the copy holds (entries on others are dropped and reported).
    pub fn permission_design(&self, keep: &dyn Fn(&str) -> bool) -> WsResult<(Value, Vec<Value>)> {
        let data = AclData::load(&self.local)?;
        let full = Access::full("system");
        let mut dropped = Vec::new();
        let mut entries = Vec::new();
        for e in &data.entries {
            if matches!(subject_kind(&e.subject), "single" | "publication") {
                continue;
            }
            let item = json!({ "subject": e.subject, "entity_id": if e.scope.is_empty() { Value::Null } else { json!(e.scope) },
                               "path": self.resource_json(&full, &e.scope)["path"], "allow": caps_json(e.allow), "deny": caps_json(e.deny) });
            if !e.scope.is_empty() && !keep(&e.scope) {
                dropped.push(item);
                continue;
            }
            entries.push(item);
        }
        let groups: Vec<Value> = data.groups.values().map(|g| json!({ "group_id": g.group_id, "title": g.title, "description": g.description })).collect();
        let mut st = self.local.prepare("SELECT wish_id, group_id, mode FROM acl_macro_bindings ORDER BY wish_id").map_err(db_err)?;
        let macros: Vec<Value> = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?
            .into_iter()
            .filter(|(w, ..)| keep(w))
            .map(|(w, g, m)| json!({ "wish_id": w, "group_id": g, "mode": m }))
            .collect();
        Ok((json!({ "groups": groups, "entries": entries, "macros": macros }), dropped))
    }

    /// Load a permission design into a new Workspace (no members, anonymous access off). Entries on entities the
    /// Workspace does not hold are skipped. Recorded as one operation.
    pub fn load_permission_design(&mut self, caller: &Caller, design: &Value) -> WsResult<Value> {
        let now = self.now();
        let groups: Vec<(String, String, String)> = design["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| (g["group_id"].as_str().unwrap_or("").to_string(), g["title"].as_str().unwrap_or("").to_string(), g["description"].as_str().unwrap_or("").to_string()))
            .collect();
        for (g, t, _) in &groups {
            if !is_group_id(g) || t.chars().count() > 128 {
                return Err(WsError::invalid_schema(format!("permission design: invalid group {g}")));
            }
        }
        let group_ids: BTreeSet<&str> = groups.iter().map(|(g, ..)| g.as_str()).collect();
        let exists = |id: &str| -> WsResult<bool> { Ok(self.ctx(&now).entity(id)?.is_some_and(|e| e.alive() && e.scope == SCOPE_SHARED)) };
        let mut entries: Vec<(String, String, CapSet, CapSet)> = Vec::new();
        let mut skipped = Vec::new();
        for e in design["entries"].as_array().into_iter().flatten() {
            let subject = e["subject"].as_str().unwrap_or("").to_string();
            let ok_subject = match subject.as_str() {
                SUBJECT_ALL | SUBJECT_GUEST | SUBJECT_CREATOR => true,
                s => s.strip_prefix('@').is_some_and(|g| group_ids.contains(g)),
            };
            if !ok_subject {
                return Err(WsError::invalid_schema(format!("permission design: {subject} is not a group or a fixed group (a design names no single caller)")));
            }
            let allow = CapSet::parse_json(e.get("allow"), "allow")?;
            let deny = CapSet::parse_json(e.get("deny"), "deny")?;
            if matches!(subject.as_str(), SUBJECT_ALL | SUBJECT_GUEST) && (allow.has(Cap::Manage) || allow.has(Cap::Run)) || subject == SUBJECT_CREATOR && allow.has(Cap::Manage) {
                return Err(WsError::invalid_schema(format!("permission design: {subject} cannot be allowed that")));
            }
            let scope = match e["entity_id"].as_str() {
                None => String::new(),
                Some(id) if id == ROOT_ID => String::new(),
                Some(id) if exists(id)? => id.to_string(),
                Some(_) => {
                    skipped.push(e.clone());
                    continue;
                }
            };
            entries.push((subject, scope, allow, deny));
        }
        let mut macros = Vec::new();
        for m in design["macros"].as_array().into_iter().flatten() {
            let (w, g, mode) = (m["wish_id"].as_str().unwrap_or(""), m["group_id"].as_str().unwrap_or(""), m["mode"].as_str().unwrap_or("union"));
            if group_ids.contains(g) && matches!(mode, "union" | "intersection") && exists(w)? {
                macros.push((w.to_string(), g.to_string(), mode.to_string()));
            }
        }
        let summary = json!({ "groups": groups.len(), "entries": entries.len(), "macros": macros.len(), "skipped": skipped });
        self.acl_txn(caller, |tx| {
            for (g, t, d) in &groups {
                tx.execute("INSERT OR REPLACE INTO acl_groups (group_id, title, description, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![g, t, d, caller.principal, now]).map_err(db_err)?;
            }
            for (s, sc, a, d) in &entries {
                tx.execute("INSERT OR REPLACE INTO acl_entries (subject, scope_entity_id, allow, deny, updated_by, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![s, sc, a.0, d.0, caller.principal, now]).map_err(db_err)?;
            }
            for (w, g, m) in &macros {
                tx.execute("INSERT OR REPLACE INTO acl_macro_bindings (wish_id, group_id, mode, set_by, set_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![w, g, m, caller.principal, now]).map_err(db_err)?;
            }
            Ok(((), vec![AuditItem { action: "acl.load_design", target: json!({}), before: None, after: Some(summary.clone()) }]))
        })?;
        Ok(summary)
    }

    /// Who an explanation, a matrix test or the CLI talks about, by kind: what the caller holds on `entity_id`, by name.
    pub fn caps_on(&self, caller: &Caller, entity_id: Option<&str>) -> WsResult<Vec<&'static str>> {
        let access = self.access(caller)?;
        let caps = match entity_id {
            Some(id) => {
                let now = self.now();
                access.caps(&self.ctx(&now), id)?
            }
            None => access.ws_caps,
        };
        Ok(ALL_CAPS.iter().filter(|(c, _)| caps.has(*c)).map(|(_, n)| *n).collect())
    }

    /// Whether `caller` is an account or External that a judgment could name (for `acl.members` of entries).
    pub fn is_person(caller: &Caller) -> bool {
        matches!(caller.class, CallerClass::Account { .. } | CallerClass::External { .. } | CallerClass::Guest)
    }
}
