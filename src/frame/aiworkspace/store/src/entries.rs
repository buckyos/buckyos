//! Share entries (hybrid SSO contract §8.3): a link into a Workspace (or an entity subtree) that says which ways in
//! it offers and what a visitor gets there. Entries live in `local.sqlite` beside the grants they lead to; an entry
//! id names its Workspace (`<workspace_id>~se_…`), so a link resolves without a service-wide index.
//!
//! While an entry open to guests is active, every caller gets its template (computed per request: anyone could come
//! in anonymously anyway). A signed-in visitor whose sign-in method the entry admits joins (`ws.join_entry`) and gets
//! a grant of its own, which closing the entry does not take back.

use crate::docdb::db_err;
use crate::workspace::{random_id, Caller, CallerClass, Workspace};
use aiworkspace_core::access::{Cap, CapSet};
use aiworkspace_core::model::ReadCtx;
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
const ENTRY_METHODS: [(u8, &str); 4] =
    [(ENTRY_GUEST, "guest"), (ENTRY_PASSWORD, "password"), (ENTRY_SELF_REGISTER, "self_register"), (ENTRY_DID_LOGIN, "did_login")];
const ENTRY_SEPARATOR: char = '~';

/// `{ guest, password, self_register, did_login }` (buckyos-api `EntryMethods`).
pub fn entry_methods_json(bits: u8) -> Value {
    Value::Object(ENTRY_METHODS.iter().map(|(bit, name)| (name.to_string(), json!(bits & bit != 0))).collect())
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
    (is_prefixed_id("ws_", ws) && is_prefixed_id("se_", local)).then_some((ws, local))
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
    pub scope_entity_id: Option<String>,
    pub capabilities: CapSet,
    pub methods: u8,
    pub state: EntryState,
    /// Unix seconds.
    pub expires_at: Option<i64>,
    pub created_by: String,
    pub created_at: i64,
}

impl Entry {
    pub fn to_json(&self) -> Value {
        json!({ "entry_id": self.entry_id, "workspace_id": self.workspace_id, "scope_entity_id": self.scope_entity_id,
                "capabilities": self.capabilities.names(), "methods": entry_methods_json(self.methods), "state": self.state.as_str(),
                "expires_at": self.expires_at, "created_by": self.created_by, "created_at": self.created_at })
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
                "SELECT entry_id, scope_entity_id, capabilities, methods, state, expires_at, created_by, created_at FROM share_entries \
                 WHERE ?1 IS NULL OR entry_id = ?1 ORDER BY created_at, entry_id",
            )
            .map_err(db_err)?;
        let now = self.now_secs();
        let rows: Vec<(String, String, u16, u8, String, Option<i64>, String, i64)> = st
            .query_map(params![local], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, scope, caps, methods, state, expires_at, created_by, created_at)| Entry {
                entry_id: self.entry_public_id(&id),
                workspace_id: self.workspace_id.clone(),
                scope_entity_id: (!scope.is_empty()).then_some(scope),
                capabilities: CapSet(caps),
                methods,
                state: match state.as_str() {
                    "active" if expires_at.is_some_and(|e| e <= now) => EntryState::Expired,
                    "active" => EntryState::Active,
                    _ => EntryState::Closed,
                },
                expires_at,
                created_by,
                created_at,
            })
            .collect())
    }

    /// The entry `local` (the part after `~`), whatever its state.
    pub fn entry(&self, local: &str) -> WsResult<Option<Entry>> {
        Ok(self.entry_rows(Some(local))?.into_iter().next())
    }

    /// What everyone gets through active entries open to guests: `(scope, caps)`.
    pub(crate) fn guest_entry_grants(&self) -> WsResult<Vec<(String, u16)>> {
        let mut st = self
            .local
            .prepare_cached(
                "SELECT scope_entity_id, capabilities FROM share_entries \
                 WHERE state = 'active' AND (methods & ?1) != 0 AND (expires_at IS NULL OR expires_at > ?2)",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![ENTRY_GUEST, self.now_secs()], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// Whether the caller manages `scope` (the Workspace when `None`).
    fn manages(&self, caller: &Caller, scope: Option<&str>) -> WsResult<bool> {
        let access = self.require_ws_any(caller)?;
        match scope {
            None => Ok(access.ws_caps.has(Cap::Manage)),
            Some(id) => {
                let now = self.now();
                access.can(&self.ctx(&now), id, Cap::Manage)
            }
        }
    }

    /// `ws.create_entry`: `manage` on the scope; the template never carries `manage`, at least one way in is offered.
    pub fn create_entry(&mut self, caller: &Caller, scope: Option<&str>, caps: &[String], methods: u8, expires_at: Option<i64>) -> WsResult<Value> {
        if !self.manages(caller, scope)? {
            return Err(WsError::denied("manage capability required"));
        }
        let caps = CapSet::parse(caps)?;
        if caps == CapSet::NONE {
            return Err(WsError::invalid_op("an entry grants at least one capability"));
        }
        if caps.has(Cap::Manage) {
            return Err(WsError::invalid_op("an entry cannot carry the manage capability"));
        }
        if methods == 0 {
            return Err(WsError::invalid_op("an entry offers at least one way in"));
        }
        let now = self.now_secs();
        if expires_at.is_some_and(|e| e <= now) {
            return Err(WsError::invalid_op("expires_at must be in the future"));
        }
        if let Some(s) = scope {
            let ts = self.now();
            if self.ctx(&ts).entity(s)?.is_none() {
                return Err(WsError::not_found(format!("entity {s} not found")));
            }
        }
        let local = random_id("se_");
        self.local
            .execute(
                "INSERT INTO share_entries (entry_id, scope_entity_id, capabilities, methods, state, expires_at, created_by, created_at) \
                 VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, ?7)",
                params![local, scope.unwrap_or(""), caps.0, methods, expires_at, caller.principal, now],
            )
            .map_err(db_err)?;
        let entry = self.entry(&local)?.ok_or_else(|| WsError::io("entry vanished"))?;
        Ok(json!({ "ok": true, "entry": entry.to_json() }))
    }

    /// `ws.list_entries`: the entries on scopes the caller manages (all of them for a Workspace manager).
    pub fn list_entries(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        if !access.ws_caps.has(Cap::Manage) && !access.scoped.values().any(|c| c.has(Cap::Manage)) {
            return Err(WsError::denied("manage capability required"));
        }
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for e in self.entry_rows(None)? {
            let manages = match &e.scope_entity_id {
                None => access.ws_caps.has(Cap::Manage),
                Some(id) => access.can(&ctx, id, Cap::Manage)?,
            };
            if manages {
                out.push(e.to_json());
            }
        }
        Ok(json!({ "ok": true, "entries": out }))
    }

    /// `ws.close_entry`: guests lose the template at their next request; grants of those who joined stay
    /// (they go with `ws.revoke`). Closing a closed entry is a no-op.
    pub fn close_entry(&mut self, caller: &Caller, local: &str) -> WsResult<Value> {
        let entry = self.entry(local)?.ok_or_else(|| WsError::not_found("entry not found"))?;
        if !self.manages(caller, entry.scope_entity_id.as_deref())? {
            return Err(WsError::denied("manage capability required"));
        }
        self.local.execute("UPDATE share_entries SET state = 'closed' WHERE entry_id = ?1", [local]).map_err(db_err)?;
        let entry = self.entry(local)?.ok_or_else(|| WsError::io("entry vanished"))?;
        Ok(json!({ "ok": true, "entry": entry.to_json() }))
    }

    /// `ws.join_entry`: a signed-in visitor (an account or an External DID) gets the entry's template on its scope
    /// as a grant of its own, merged into what it already has there. `allowed` decides whether the entry's methods
    /// admit the caller's session (sign-in method ∩ Zone switches, judged by the service). An entry open to guests
    /// that does not admit the session still lets it in, as it lets anyone in: no grant is written (`granted: false`)
    /// and the access ends with the entry. Joining again changes nothing; nobody becomes the owner by joining.
    pub fn join_entry(&mut self, caller: &Caller, local: &str, allowed: impl FnOnce(u8) -> WsResult<()>) -> WsResult<Value> {
        if !matches!(caller.class, CallerClass::Account { .. } | CallerClass::External { .. }) {
            return Err(WsError::refused(Code::PermissionDenied, Deny::SessionKindNotAllowed, "only a signed-in person joins an entry"));
        }
        let entry = self.entry(local)?.filter(|e| e.state == EntryState::Active).ok_or_else(unavailable)?;
        if let Err(refused) = allowed(entry.methods) {
            if entry.methods & ENTRY_GUEST == 0 {
                return Err(refused);
            }
            return Ok(json!({ "ok": true, "workspace_id": self.workspace_id, "entry_id": entry.entry_id, "scope_entity_id": entry.scope_entity_id,
                              "capabilities": entry.capabilities.names(), "joined": false, "granted": false }));
        }
        let scope = entry.scope_entity_id.clone().unwrap_or_default();
        let existing: Option<u16> = self
            .local
            .query_row("SELECT capabilities FROM grants WHERE subject = ?1 AND scope_entity_id = ?2", params![caller.principal, scope], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        let caps = CapSet(existing.unwrap_or(0)).union(entry.capabilities);
        self.local
            .execute(
                "INSERT INTO grants (subject, scope_entity_id, capabilities, via_entry) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (subject, scope_entity_id) DO UPDATE SET capabilities = excluded.capabilities",
                params![caller.principal, scope, caps.0, local],
            )
            .map_err(db_err)?;
        Ok(json!({ "ok": true, "workspace_id": self.workspace_id, "entry_id": entry.entry_id, "scope_entity_id": entry.scope_entity_id,
                   "capabilities": caps.names(), "joined": existing != Some(caps.0), "granted": true }))
    }
}
