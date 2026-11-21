//! Share entries (hybrid SSO contract §8.3, 权限需求 R8): a link into a Workspace that says which ways in it offers and
//! which logical group a visitor joins there; what the group may do is what its entries say. Entries live in
//! `local.sqlite` beside the groups they lead into; an entry id names its Workspace (`<workspace_id>~se_…`), so a link
//! resolves without a service-wide index.
//!
//! Nobody gets anything without presenting the entry: a visitor — signed in, or a guest with its browser's token —
//! joins (`ws.join_entry`) and becomes a member of the target group (`via_entry`). Closing an entry stops new joins and
//! may remove those who joined through it.

use crate::acl::AuditItem;
use crate::docdb::db_err;
use crate::workspace::{random_id, Caller, CallerClass, Workspace};
use aiworkspace_core::access::Cap;
use aiworkspace_core::id::is_prefixed_id;
use aiworkspace_core::{Code, Deny, WsError, WsResult};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub const ENTRY_GUEST: u8 = 1;
/// An existing local account of the Zone signs in.
pub const ENTRY_PASSWORD: u8 = 2;
/// A visitor registers a local account (implies `password`).
pub const ENTRY_SELF_REGISTER: u8 = 4;
pub const ENTRY_DID_LOGIN: u8 = 8;
pub const ENTRY_OAUTH: u8 = 16;
const ENTRY_METHODS: [(u8, &str); 5] =
    [(ENTRY_GUEST, "guest"), (ENTRY_PASSWORD, "password"), (ENTRY_SELF_REGISTER, "self_register"), (ENTRY_DID_LOGIN, "did_login"), (ENTRY_OAUTH, "oauth")];
const ENTRY_SEPARATOR: char = '~';

/// `{ guest, password, self_register, did_login }` (buckyos-api `EntryMethods`).
pub fn entry_methods_json(bits: u8) -> Value {
    Value::Object(ENTRY_METHODS.iter()
        .filter(|(bit, _)| *bit != ENTRY_OAUTH || bits & bit != 0)
        .map(|(bit, name)| (name.to_string(), json!(bits & bit != 0)))
        .collect())
}

pub fn parse_entry_methods(v: &Value) -> WsResult<u8> {
    let o = v.as_object().ok_or_else(|| WsError::invalid_op("methods must be an object"))?;
    let mut bits = 0;
    for (key, value) in o {
        let bit = ENTRY_METHODS.iter().find(|(_, name)| name == key).ok_or_else(|| WsError::invalid_op(format!("unknown entry method {key}")))?.0;
        match value.as_bool() {
            Some(true) => bits |= bit,
            Some(false) => {}
            None => return Err(WsError::invalid_op(format!("methods.{key} must be a boolean"))),
        }
    }
    if bits & ENTRY_SELF_REGISTER != 0 {
        bits |= ENTRY_PASSWORD;
    }
    Ok(bits)
}

/// `<workspace_id>~se_…` → (workspace id, entry id within it).
pub fn split_entry_id(entry_id: &str) -> Option<(&str, &str)> {
    let (ws, local) = entry_id.split_once(ENTRY_SEPARATOR)?;
    (aiworkspace_core::id::is_workspace_id(ws) && is_prefixed_id("se_", local)).then_some((ws, local))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryState {
    Active,
    Closed,
    Expired,
}

impl EntryState {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryState::Active => "active",
            EntryState::Closed => "closed",
            EntryState::Expired => "expired",
        }
    }
}

/// One stored entry as the service sees it.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The public id (`<workspace_id>~se_…`).
    pub entry_id: String,
    pub workspace_id: String,
    /// The logical group a visitor joins.
    pub group_id: String,
    pub methods: u8,
    pub state: EntryState,
    /// Unix seconds.
    pub expires_at: Option<i64>,
    /// How many may join through it at most (an anonymous entry's limit).
    pub max_joins: Option<i64>,
    pub created_by: String,
    pub created_at: i64,
}

impl Entry {
    pub fn to_json(&self) -> Value {
        json!({ "entry_id": self.entry_id, "workspace_id": self.workspace_id, "group_id": self.group_id,
                "methods": entry_methods_json(self.methods), "state": self.state.as_str(), "expires_at": self.expires_at,
                "max_joins": self.max_joins, "created_by": self.created_by, "created_at": self.created_at })
    }
}

fn unavailable() -> WsError {
    WsError::refused(Code::NotFound, Deny::EntryUnavailable, "the entry is closed, expired or does not exist")
}

impl Workspace {
    fn now_secs(&self) -> i64 {
        (self.clock)() / 1000
    }

    pub(crate) fn entry_public_id(&self, local: &str) -> String {
        format!("{}{ENTRY_SEPARATOR}{local}", self.workspace_id)
    }

    fn entry_rows(&self, local: Option<&str>) -> WsResult<Vec<Entry>> {
        let mut st = self
            .local
            .prepare_cached(
                "SELECT entry_id, group_id, methods, state, expires_at, max_joins, created_by, created_at FROM share_entries \
                 WHERE ?1 IS NULL OR entry_id = ?1 ORDER BY created_at, entry_id",
            )
            .map_err(db_err)?;
        let now = self.now_secs();
        let rows: Vec<(String, String, u8, String, Option<i64>, Option<i64>, String, i64)> = st
            .query_map(params![local], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, group_id, methods, state, expires_at, max_joins, created_by, created_at)| Entry {
                entry_id: self.entry_public_id(&id),
                workspace_id: self.workspace_id.clone(),
                group_id,
                methods,
                state: match state.as_str() {
                    "active" if expires_at.is_some_and(|e| e <= now) => EntryState::Expired,
                    "active" => EntryState::Active,
                    _ => EntryState::Closed,
                },
                expires_at,
                max_joins,
                created_by,
                created_at,
            })
            .collect())
    }

    /// The entry `local` (the part after `~`), whatever its state.
    pub fn entry(&self, local: &str) -> WsResult<Option<Entry>> {
        Ok(self.entry_rows(Some(local))?.into_iter().next())
    }

    /// `ws.create_entry { group_id, methods, expires_at?, max_joins? }` (`manage`): at least one way in; the group exists
    /// and holds neither `manage` nor `run` (R8, R18).
    pub fn create_entry(&mut self, caller: &Caller, group_id: &str, methods: u8, expires_at: Option<i64>, max_joins: Option<i64>) -> WsResult<Value> {
        self.create_entry_with_registration(caller,group_id,methods,expires_at,max_joins,None)
    }

    fn create_entry_with_registration(&mut self, caller: &Caller, group_id: &str, methods: u8, expires_at: Option<i64>, max_joins: Option<i64>, registration: Option<(&str,&Value,&str)>) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let data = self.acl_data()?;
        if !data.groups.contains_key(group_id) {
            return Err(WsError::not_found(format!("group {group_id} not found")));
        }
        let held = data.allowed_anywhere(&format!("@{group_id}"));
        if held.has(Cap::Manage) || held.has(Cap::Run) {
            return Err(WsError::invalid_op("an entry cannot lead into a group holding manage or run (权限需求 R8, R18)"));
        }
        if methods == 0 {
            return Err(WsError::invalid_op("an entry offers at least one way in"));
        }
        let now = self.now_secs();
        if expires_at.is_some_and(|e| e <= now) {
            return Err(WsError::invalid_op("expires_at must be in the future"));
        }
        if max_joins.is_some_and(|m| !(1..=100_000).contains(&m)) {
            return Err(WsError::invalid_op("max_joins must be 1…100000"));
        }
        let local = random_id("se_");
        self.acl_txn(caller, |tx| {
            tx.execute(
                "INSERT INTO share_entries (entry_id, group_id, methods, state, expires_at, max_joins, created_by, created_at) VALUES (?1, ?2, ?3, 'active', ?4, ?5, ?6, ?7)",
                params![local, group_id, methods, expires_at, max_joins, caller.principal, now],
            )
            .map_err(db_err)?;
            if let Some((request_id, request, secret)) = registration {
                tx.execute("INSERT INTO registered_entries(entry_id,request_id,creator,request,secret) VALUES(?1,?2,?3,?4,?5)",
                    params![local,request_id,caller.principal,request.to_string(),secret]).map_err(db_err)?;
            }
            let after = json!({ "entry_id": self.entry_public_id(&local), "group_id": group_id, "methods": entry_methods_json(methods), "expires_at": expires_at, "max_joins": max_joins });
            Ok(((), vec![AuditItem { action: "entry.create", target: json!({ "entry_id": self.entry_public_id(&local) }), before: None, after: Some(after) }]))
        })?;
        let entry = self.entry(&local)?.ok_or_else(|| WsError::io("entry vanished"))?;
        Ok(json!({ "ok": true, "entry": entry.to_json() }))
    }

    /// `ws.list_entries` (`manage`): every entry, with how many joined through each.
    pub fn list_entries(&self, caller: &Caller) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let mut out = Vec::new();
        for e in self.entry_rows(None)? {
            let local = e.entry_id.split_once('~').map(|(_, l)| l.to_string()).unwrap_or_default();
            let joined: i64 = self.local.query_row("SELECT COUNT(*) FROM acl_members WHERE via_entry = ?1", [&local], |r| r.get(0)).map_err(db_err)?;
            let mut v = e.to_json();
            let registration: Option<Option<String>> = self.local.query_row("SELECT share_url FROM registered_entries WHERE entry_id=?1",[&local],|r| r.get(0)).optional().map_err(db_err)?;
            if let Some(url) = registration {
                v["share_url"] = json!(url);
                let count:i64 = self.local.query_row("SELECT COUNT(*) FROM (SELECT member FROM entry_acceptances WHERE entry_id=?1 AND committed=1 UNION SELECT member FROM acl_members WHERE via_entry=?1)",[&local],|r|r.get(0)).map_err(db_err)?;
                v["joined"] = json!(count);
            } else {
                v["joined"] = json!(joined);
            }
            out.push(v);
        }
        Ok(json!({ "ok": true, "entries": out }))
    }

    /// `ws.close_entry { entry_id, remove_members? }` (`manage`): nobody joins through it any more; with
    /// `remove_members` those who joined through it leave the group too. Closing a closed entry changes nothing else.
    pub fn close_entry(&mut self, caller: &Caller, local: &str, remove_members: bool) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let entry = self.entry(local)?.ok_or_else(|| WsError::not_found("entry not found"))?;
        let (removed, _) = self.acl_txn(caller, |tx| {
            tx.execute("UPDATE share_entries SET state = 'closed' WHERE entry_id = ?1", [local]).map_err(db_err)?;
            let removed = if remove_members {
                tx.execute("DELETE FROM entry_acceptances WHERE entry_id=?1",[local]).map_err(db_err)?;
                tx.execute("UPDATE acl_members SET via_entry=(SELECT MIN(a.entry_id) FROM entry_acceptances a JOIN share_entries e ON e.entry_id=a.entry_id WHERE a.member=acl_members.member AND e.group_id=acl_members.group_id AND a.committed=1) WHERE via_entry=?1 AND EXISTS(SELECT 1 FROM entry_acceptances a JOIN share_entries e ON e.entry_id=a.entry_id WHERE a.member=acl_members.member AND e.group_id=acl_members.group_id AND a.committed=1)",[local]).map_err(db_err)?;
                tx.execute("DELETE FROM acl_members WHERE via_entry = ?1", [local]).map_err(db_err)?
            } else { 0 };
            Ok((removed, vec![AuditItem { action: "entry.close", target: json!({ "entry_id": entry.entry_id, "group_id": entry.group_id }),
                                         before: Some(json!({ "state": entry.state.as_str() })), after: Some(json!({ "state": "closed", "removed_members": removed })) }]))
        })?;
        let entry = self.entry(local)?.ok_or_else(|| WsError::io("entry vanished"))?;
        Ok(json!({ "ok": true, "entry": entry.to_json(), "removed_members": removed }))
    }

    /// `ws.join_entry`: the visitor becomes a member of the entry's group (`via_entry`). A signed-in visitor joins when
    /// the entry's methods admit its session (`allowed`: sign-in method ∩ Zone switches, judged by the service), or
    /// when the entry is open to guests (anyone may come in anonymously anyway); a guest (its browser's token) joins
    /// an entry open to guests. Joining again changes nothing; nobody becomes the owner by joining.
    pub fn join_entry(&mut self, caller: &Caller, local: &str, allowed: impl FnOnce(u8) -> WsResult<()>) -> WsResult<Value> {
        let entry = self.entry(local)?.filter(|e| e.state == EntryState::Active).ok_or_else(unavailable)?;
        let awaiting_registration: bool = self.local.query_row(
            "SELECT EXISTS(SELECT 1 FROM registered_entries WHERE entry_id=?1 AND share_url IS NULL)",
            [local], |r| r.get(0),
        ).map_err(db_err)?;
        if awaiting_registration { return Err(unavailable()); }
        match caller.class {
            CallerClass::Account { .. } | CallerClass::External { .. } => {
                if let Err(refused) = allowed(entry.methods) {
                    if entry.methods & ENTRY_GUEST == 0 {
                        return Err(refused);
                    }
                }
            }
            CallerClass::Guest if caller.publication.is_none() && !caller.principal.starts_with("link:") => {
                if entry.methods & ENTRY_GUEST == 0 {
                    return Err(WsError::refused(Code::PermissionDenied, Deny::EntryUnavailable, "the entry is not open to guests: sign in"));
                }
            }
            _ => return Err(WsError::refused(Code::PermissionDenied, Deny::SessionKindNotAllowed, "only a person joins an entry")),
        }
        let already: bool = self
            .local
            .query_row("SELECT 1 FROM acl_members WHERE group_id = ?1 AND member = ?2", params![entry.group_id, caller.principal], |_| Ok(true))
            .optional()
            .map_err(db_err)?
            .unwrap_or(false);
        if !already {
            if let Some(max) = entry.max_joins {
                let joined: i64 = self.local.query_row("SELECT COUNT(*) FROM (SELECT member FROM acl_members WHERE via_entry=?1 UNION SELECT member FROM entry_acceptances WHERE entry_id=?1 AND (committed=1 OR reserved_until>?2))", params![local,self.now_secs()], |r| r.get(0)).map_err(db_err)?;
                if joined >= max {
                    return Err(WsError::refused(Code::PermissionDenied, Deny::EntryUnavailable, "the entry admits nobody else"));
                }
            }
            let now = self.now();
            self.acl_txn(caller, |tx| {
                tx.execute("INSERT INTO acl_members (group_id, member, via_entry, added_by, added_at) VALUES (?1, ?2, ?3, ?2, ?4)", params![entry.group_id, caller.principal, local, now])
                    .map_err(db_err)?;
                Ok(((), vec![AuditItem { action: "entry.join", target: json!({ "entry_id": entry.entry_id, "group_id": entry.group_id }), before: None, after: Some(json!({ "member": caller.principal })) }]))
            })?;
        }
        Ok(json!({ "ok": true, "workspace_id": self.workspace_id, "entry_id": entry.entry_id, "group_id": entry.group_id, "joined": !already }))
    }
}

pub const REGISTERED_ENTRIES_DDL: &str = "
CREATE TABLE IF NOT EXISTS registered_entries (
  entry_id TEXT PRIMARY KEY, request_id TEXT NOT NULL, creator TEXT NOT NULL,
  request TEXT NOT NULL, secret TEXT NOT NULL, share_url TEXT,
  UNIQUE(creator, request_id)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS entry_acceptances (
  entry_id TEXT NOT NULL, member TEXT NOT NULL, reserved_until INTEGER NOT NULL, committed INTEGER NOT NULL,
  PRIMARY KEY(entry_id, member)
) WITHOUT ROWID;";

impl Workspace {
    pub fn share_request(
        &mut self,
        caller: &Caller,
        request_id: &str,
        request: &Value,
        group: &str,
        methods: u8,
        expiry: Option<i64>,
        maximum: Option<i64>,
    ) -> WsResult<(String, String, Option<String>)> {
        self.require_ws(caller, Cap::Manage)?;
        if let Some((id, secret, old, url)) = self.local.query_row(
            "SELECT entry_id, secret, request, share_url FROM registered_entries WHERE creator=?1 AND request_id=?2",
            params![caller.principal,request_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?)))
            .optional().map_err(db_err)? {
            if old != request.to_string() { return Err(WsError::invalid_op("share request_id was already used")); }
            return Ok((self.entry_public_id(&id),secret,url));
        }
        let secret = format!("{}{}", random_id("sc_"), random_id(""));
        let value = self.create_entry_with_registration(
            caller,
            group,
            methods,
            expiry,
            maximum,
            Some((request_id, request, &secret)),
        )?;
        let id = value["entry"]["entry_id"]
            .as_str()
            .ok_or_else(unavailable)?;
        Ok((id.to_owned(), secret, None))
    }

    pub fn save_share_url(&self, local: &str, url: &str) -> WsResult<()> {
        self.local
            .execute(
                "UPDATE registered_entries SET share_url=?2 WHERE entry_id=?1",
                params![local, url],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn check_share_callback(&self, local: &str, secret: &str, active: bool) -> WsResult<Entry> {
        let matches: bool = self
            .local
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM registered_entries WHERE entry_id=?1 AND secret=?2)",
                params![local, secret],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        if !matches {
            return Err(unavailable());
        }
        self.entry(local)?
            .filter(|e| !active || e.state == EntryState::Active)
            .ok_or_else(unavailable)
    }

    pub fn has_share_grant(&self, local: &str, entry: &Entry, did: &str) -> WsResult<bool> {
        let acl = self.acl_data()?;
        if !acl.groups.contains_key(&entry.group_id) || acl.allowed_anywhere(&format!("@{}", entry.group_id)).is_empty() {
            return Ok(false);
        }
        self.local.query_row("SELECT EXISTS(SELECT 1 FROM entry_acceptances a JOIN acl_members m ON m.group_id=?2 AND m.member=a.member WHERE a.entry_id=?1 AND a.member=?3 AND a.committed=1)",
            params![local,entry.group_id,did],|r|r.get(0)).map_err(db_err)
    }

    pub fn share_grant(
        &mut self,
        local: &str,
        entry: &Entry,
        did: &str,
        commit: bool,
    ) -> WsResult<()> {
        let now = self.now_secs();
        let caller = Caller::external(did);
        self.acl_txn(&caller, |tx| {
            // 0 is reserved, 1 is committed, and -1 records an explicit membership revocation.
            let committed: Option<i64> = tx.query_row("SELECT committed FROM entry_acceptances WHERE entry_id=?1 AND member=?2",
                params![local,did],|r| r.get(0)).optional().map_err(db_err)?;
            if committed == Some(-1) {
                return Err(WsError::denied("resource authorization was removed"));
            }
            if committed == Some(1) {
                let present: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM acl_members WHERE group_id=?1 AND member=?2)", params![entry.group_id,did],|r| r.get(0)).map_err(db_err)?;
                if !present { return Err(WsError::denied("resource authorization was removed")); }
                return Ok(((),vec![]));
            }
            if let Some(max) = entry.max_joins {
                let count:i64 = tx.query_row("SELECT COUNT(*) FROM (SELECT member FROM entry_acceptances WHERE entry_id=?1 AND member!=?2 AND (committed=1 OR reserved_until>?3) UNION SELECT member FROM acl_members WHERE via_entry=?1 AND member!=?2)",
                    params![local,did,now],|r| r.get(0)).map_err(db_err)?;
                if count >= max { return Err(unavailable()); }
            }
            if commit && committed.is_none() { return Err(unavailable()); }
            tx.execute("INSERT INTO entry_acceptances VALUES(?1,?2,?3,?4) ON CONFLICT(entry_id,member) DO UPDATE SET reserved_until=excluded.reserved_until,committed=excluded.committed",
                params![local,did,now+60,commit]).map_err(db_err)?;
            if !commit { return Ok(((),vec![])); }
            tx.execute("INSERT OR IGNORE INTO acl_members (group_id, member, via_entry, added_by, added_at) VALUES (?1,?2,?3,?2,?4)",
                params![entry.group_id,did,local,self.now()]).map_err(db_err)?;
            Ok(((),vec![AuditItem{action:"entry.join",target:json!({"entry_id":entry.entry_id,"group_id":entry.group_id}),before:None,after:Some(json!({"member":did}))}]))
        })?;
        Ok(())
    }

}
