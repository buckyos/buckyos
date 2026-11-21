use super::{local_get, local_set, Workspace, SOURCE_PROGRAM};
use crate::docdb::db_err;
use crate::{Caller, CommitOpts};
use aiworkspace_core::access::Cap;
use aiworkspace_core::id::check_idempotency_key;
use aiworkspace_core::model::{EntityRow, ReadCtx, PROTOCOL_VERSION, TYPE_TABLE};
use aiworkspace_core::plan::CommitFailure;
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Binding {
    workspace_id: String,
    epoch: String,
    entity_id: String,
    owner: String,
    did: String,
    pub(super) generation: String,
    source_rev: u64,
    life_rev: u64,
    pub(super) staged_rev: Option<u64>,
    pub(super) source: Value,
    pub(super) once: bool,
    activation: Option<(String, String)>,
}

pub(super) fn is_contacts(source: &Value) -> bool {
    source["kind"] == "builtin" && source["provider"] == "contacts"
}

#[derive(Serialize, Deserialize)]
struct ModeChange {
    request: Value,
    nonce: String,
    original: Option<Binding>,
    replacement: Option<Binding>,
    source_rev: u64,
    life_rev: u64,
}

fn binding_key(entity_id: &str) -> String {
    format!("contacts.binding/{entity_id}")
}

impl Binding {
    fn load(db: &Connection, entity_id: &str) -> WsResult<Option<Self>> {
        local_get(db, &binding_key(entity_id))?.map(|s| serde_json::from_str(&s).map_err(|e| WsError::io(e.to_string()))).transpose()
    }

    pub(super) fn save(&self, db: &Connection) -> WsResult<()> {
        local_set(db, &binding_key(&self.entity_id), &serde_json::to_string(self).map_err(|e| WsError::io(e.to_string()))?)
    }
}

fn needs_rebind() -> WsError {
    WsError::sub(Code::PermissionDenied, "CONTACTS_BINDING_REQUIRED", "the owner must bind this table to their Zone contacts")
}

impl Workspace {
    pub(crate) fn check_contacts_undo(&self, ops: &[Value], is_owner: bool) -> Result<(), CommitFailure> {
        fn keys(op: &Value) -> impl Iterator<Item = &Value> {
            op["keys"].as_array().into_iter().flatten()
        }
        let attaching: std::collections::BTreeSet<&str> = ops
            .iter()
            .filter_map(|op| {
                let contacts = match op["op"].as_str() {
                    Some("entity.create") => is_contacts(&op["payload"]["source"]),
                    Some("entity.set_keys") => keys(op).any(|k| k["key"] == "source" && is_contacts(&k["value"])),
                    _ => false,
                };
                op["entity_id"].as_str().filter(|_| contacts)
            })
            .collect();
        let now = self.now();
        let ctx = self.ctx(&now);
        for (index, op) in ops.iter().enumerate() {
            let name = op["op"].as_str().unwrap_or("");
            let table_write = name.starts_with("table.");
            let entity_keys = matches!(name, "entity.set_keys" | "entity.unset_keys");
            let source_write =
                (entity_keys && keys(op).any(|k| k["key"] == "source")) || (name == "entity.create" && is_contacts(&op["payload"]["source"]));
            let version_write = entity_keys && keys(op).any(|k| k["key"] == "data_version");
            if !table_write && !source_write && !version_write {
                continue;
            }
            let Some(id) = op[if table_write { "source_id" } else { "entity_id" }].as_str() else { continue };
            let reject = |error| CommitFailure::Rejected { error, op_index: Some(index) };
            if !attaching.contains(id) && !ctx.entity(id).map_err(reject)?.is_some_and(|e| e.payload.get("source").is_some_and(is_contacts)) {
                continue;
            }
            if source_write && !is_owner {
                return Err(reject(WsError::denied("only the workspace owner changes the contacts source")));
            }
            if table_write || version_write {
                return Err(reject(
                    WsError::sub(Code::InvalidOperation, "SOURCE_READ_ONLY", "a synced contacts table is written by its refresh")
                        .with_data(json!({ "entity_id": id })),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn contact_binding(&self, e: &EntityRow) -> WsResult<Option<Binding>> {
        let Some(b) = Binding::load(&self.local, &e.entity_id)? else { return Ok(None) };
        Ok(self.contact_authorized(e, &b)?.then_some(b))
    }

    fn contact_authorized(&self, e: &EntityRow, b: &Binding) -> WsResult<bool> {
        let activated = match &b.activation {
            None => true,
            Some((principal, nonce)) => self.contact_config_seq(principal, nonce)? == Some(b.source_rev),
        };
        let valid = activated
            && e.alive()
            && e.type_id == TYPE_TABLE
            && b.workspace_id == self.workspace_id
            && b.epoch == self.epoch
            && b.entity_id == e.entity_id
            && self.owner()?.as_ref() == Some(&b.owner)
            && e.life_rev == b.life_rev
            && e.key_rev("source") == b.source_rev
            && match b.staged_rev {
                Some(rev) => e.content_rev == rev,
                None => e.payload.get("source") == Some(&b.source),
            };
        Ok(valid)
    }

    pub(crate) fn contact_metadata(&self, e: &EntityRow, data: &mut Value) -> WsResult<()> {
        let builtin = e.payload.get("source").is_some_and(is_contacts);
        if builtin {
            data["binding"] = json!("needs_rebind");
        }
        let Some(b) = Binding::load(&self.local, &e.entity_id)? else { return Ok(()) };
        if !builtin
            && b.staged_rev.is_some()
            && b.workspace_id == self.workspace_id
            && b.epoch == self.epoch
            && e.life_rev == b.life_rev
            && e.key_rev("source") == b.source_rev
        {
            data["attempt"] = self.source_attempt(&e.entity_id)?.unwrap_or(Value::Null);
        }
        if self.contact_authorized(e, &b)? {
            data["binding"] = json!("bound");
            data["owner_did"] = json!(b.did);
            if b.once {
                data["localize_after_refresh"] = json!(true);
            }
        }
        Ok(())
    }

    pub fn bind_contacts(
        &mut self,
        caller: &Caller,
        entity_id: &str,
        owner_did: &str,
        source: &Value,
        once: bool,
        expect_content_rev: u64,
        idem: &str,
    ) -> WsResult<Value> {
        self.require_owner(caller)?;
        self.require_on(caller, Some(entity_id), Cap::Update)?;
        check_idempotency_key(idem)?;
        aiworkspace_core::types::check_source(source)?;
        if !is_contacts(source) || owner_did.is_empty() {
            return Err(WsError::invalid_op("a Zone contacts binding needs its resolved owner and contacts source"));
        }
        let owner = self.owner()?.ok_or_else(needs_rebind)?;
        let request = json!({ "entity_id": entity_id, "source": source, "once": once, "content_rev": expect_content_rev, "did": owner_did, "epoch": self.epoch });
        if let Some(run_id) = self.source_idempotent_run(&caller.principal, idem)? {
            let mut run = self.run_json(&run_id)?;
            if run["params"]["bind_request"] != request {
                return Err(WsError::new(Code::IdempotencyMismatch, "binding request changed"));
            }
            run["ok"] = json!(true);
            run["replayed"] = json!(true);
            return Ok(run);
        }
        let e = self.ctx(&self.now()).entity(entity_id)?.filter(|e| e.alive()).ok_or_else(|| WsError::not_found("table not found"))?;
        if e.type_id != TYPE_TABLE || aiworkspace_core::types::is_url_table(&e) || e.payload.get("source").is_some_and(|s| !is_contacts(s)) {
            return Err(WsError::invalid_op("Zone contacts require a local or contacts table"));
        }
        let receipt_key =
            format!("contacts.mode/{}", aiworkspace_core::canonical::sha256_hex(json!([caller.principal, idem]).to_string().as_bytes()));
        let receipt: Option<ModeChange> =
            local_get(&self.local, &receipt_key)?.map(|s| serde_json::from_str(&s).map_err(|e| WsError::io(e.to_string()))).transpose()?;
        if let Some(receipt) = receipt {
            if receipt.request != request {
                return Err(WsError::new(Code::IdempotencyMismatch, "binding request changed"));
            }
            return self.apply_contacts_mode(caller, &receipt_key, receipt, &e);
        }
        let active = self.contact_binding(&e)?.filter(|b| b.did == owner_did && b.staged_rev.is_none() && !b.once);
        if e.content_rev != expect_content_rev {
            return Err(WsError::new(Code::RevisionConflict, "the table changed since the binding was confirmed"));
        }
        let configure = active.is_some() && !once && e.payload.contains_key("data_version");
        let localize = once && e.payload.contains_key("data_version") && e.payload.get("source").is_some_and(is_contacts);
        if configure || localize {
            let nonce = super::random_id("contacts-mode/");
            let original = Binding::load(&self.local, entity_id)?;
            let replacement = active.filter(|_| !localize).map(|mut b| {
                b.generation = super::random_id("bind_");
                b.source = source.clone();
                b.activation = Some((caller.principal.clone(), nonce.clone()));
                b
            });
            let receipt = ModeChange { request, nonce, original, replacement, source_rev: e.key_rev("source"), life_rev: e.life_rev };
            return self.apply_contacts_mode(caller, &receipt_key, receipt, &e);
        }
        let b = Binding {
            workspace_id: self.workspace_id.clone(),
            epoch: self.epoch.clone(),
            entity_id: entity_id.to_string(),
            owner,
            did: owner_did.to_string(),
            generation: super::random_id("bind_"),
            life_rev: e.life_rev,
            source_rev: e.key_rev("source"),
            staged_rev: (e.payload.get("source") != Some(source) || e.payload.contains_key("data_version")).then_some(e.content_rev),
            source: source.clone(),
            once,
            activation: None,
        };
        let params = json!({ "entity_id": entity_id, "source": source, "source_rev": e.key_rev("source"), "life_rev": e.life_rev,
            "epoch": self.epoch, "binding": b.generation, "bind_request": request });
        let now = self.now();
        let tx = self.local.transaction().map_err(db_err)?;
        b.save(&tx)?;
        let run_id = Self::insert_source_run(&tx, &caller.principal, idem, &params, &now)?;
        tx.commit().map_err(db_err)?;
        let mut run = self.run_json(&run_id)?;
        run["ok"] = json!(true);
        Ok(run)
    }

    fn contact_config_seq(&self, principal: &str, nonce: &str) -> WsResult<Option<u64>> {
        self.doc
            .query_row("SELECT seq FROM commits WHERE principal = ?1 AND idem_key = ?2", params![principal, nonce], |r| r.get(0))
            .optional()
            .map_err(db_err)
    }

    fn apply_contacts_mode(&mut self, caller: &Caller, receipt_key: &str, mut change: ModeChange, e: &EntityRow) -> WsResult<Value> {
        let id = &e.entity_id;
        let result = json!({ "ok": true, "entity_id": id, "configured": true });
        if self.contact_config_seq(&caller.principal, &change.nonce)?.is_some() {
            return Ok(result);
        }
        let stored = Binding::load(&self.local, id)?;
        let generation = |b: &Option<Binding>| b.as_ref().map(|b| b.generation.clone());
        if Some(e.content_rev) != change.request["content_rev"].as_u64()
            || e.key_rev("source") != change.source_rev
            || e.life_rev != change.life_rev
            || (generation(&stored) != generation(&change.original) && generation(&stored) != generation(&change.replacement))
        {
            return Err(WsError::new(Code::RevisionConflict, "the table or contacts authorization changed"));
        }
        let tx = self.local.transaction().map_err(db_err)?;
        let operation = if let Some(b) = change.replacement.as_mut() {
            b.source_rev = self.head_seq + 1;
            b.save(&tx)?;
            json!({ "op": "entity.set_keys", "entity_id": id, "keys": [{ "key": "source", "value": b.source, "expect": { "rev": change.source_rev } }] })
        } else {
            json!({ "op": "entity.unset_keys", "entity_id": id, "keys": [{ "key": "source", "expect": { "rev": change.source_rev } }] })
        };
        local_set(&tx, receipt_key, &serde_json::to_string(&change).map_err(|e| WsError::io(e.to_string()))?)?;
        tx.commit().map_err(db_err)?;
        let commit = self.commit(
            &json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch,
            "idempotency_key": change.nonce, "operations": [operation] }),
            caller,
            &CommitOpts::default(),
        );
        if commit["status"] == "accepted" || self.contact_config_seq(&caller.principal, &change.nonce)?.is_some() {
            return Ok(result);
        }
        if let Some(original) = change.original {
            original.save(&self.local)?;
        }
        Err(WsError::invalid_op("could not change contacts sync mode").with_data(commit))
    }

    pub(super) fn binding_for_run(&self, run: &Value, e: &EntityRow) -> WsResult<Binding> {
        let b = self.contact_binding(e)?.ok_or_else(needs_rebind)?;
        if Some(b.generation.as_str()) != run["params"]["binding"].as_str() {
            return Err(needs_rebind());
        }
        Ok(b)
    }

    fn current_contact_run(&self, run_id: &str) -> WsResult<(EntityRow, Binding)> {
        let run = self.run_json(run_id)?;
        if run["state"] != "queued" && run["state"] != "running" {
            return Err(WsError::new(Code::RunRevoked, "contacts run is no longer active"));
        }
        let e = self.ctx(&self.now()).entity(run["params"]["entity_id"].as_str().unwrap_or(""))?.ok_or_else(needs_rebind)?;
        let b = self.binding_for_run(&run, &e)?;
        Ok((e, b))
    }

    pub fn contact_run_is_current(&self, run_id: &str) -> WsResult<()> {
        self.current_contact_run(run_id).map(|_| ())
    }

    pub fn contact_read_context(&self, run_id: &str) -> WsResult<(String, String, Vec<String>)> {
        let (e, b) = self.current_contact_run(run_id)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut dids = Vec::new();
        if e.payload.get("source").is_some_and(is_contacts) && e.payload.contains_key("data_version") {
            ctx.scan_records(&e.entity_id, &mut |r| {
                if r.alive() {
                    if let Some(did) = r.values.get("did").and_then(Value::as_str) {
                        dids.push(did.to_owned());
                    }
                }
                Ok(true)
            })?;
        }
        Ok((b.owner, b.did, dids))
    }

    pub fn invalidate_contact_run_binding(&self, run_id: &str) -> WsResult<()> {
        let run = self.run_json(run_id)?;
        let id = run["params"]["entity_id"].as_str().unwrap_or("");
        if let Some(b) = Binding::load(&self.local, id)? {
            if Some(b.generation.as_str()) == run["params"]["binding"].as_str() {
                self.local.execute("DELETE FROM local_meta WHERE key = ?1", [binding_key(id)]).map_err(db_err)?;
            }
        }
        Ok(())
    }

    pub(super) fn finish_contact_binding(&self, run: &Value, seq: u64) -> WsResult<()> {
        let id = run["params"]["entity_id"].as_str().unwrap_or("");
        if let Some(mut b) = Binding::load(&self.local, id)? {
            if Some(b.generation.as_str()) == run["params"]["binding"].as_str() {
                if b.once {
                    self.invalidate_contact_run_binding(run["run_id"].as_str().unwrap_or(""))?;
                } else if b.staged_rev.is_some() {
                    b.source_rev = seq;
                    b.staged_rev = None;
                    b.save(&self.local)?;
                }
            }
        }
        Ok(())
    }

    pub fn resume_contact_runs(&self) -> WsResult<Vec<String>> {
        let mut st = self
            .local
            .prepare("SELECT run_id FROM runs WHERE program = ?1 AND state = 'interrupted' AND json_type(params_json, '$.binding') = 'text'")
            .map_err(db_err)?;
        let ids = st.query_map([SOURCE_PROGRAM], |r| r.get::<_, String>(0)).map_err(db_err)?.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?;
        drop(st);
        let mut resume = Vec::new();
        for id in ids {
            let run = self.run_json(&id)?;
            if let Some((result, at)) = self.committed_source_run(&id)? {
                self.source_succeeded(&run, &result, &at)?;
            } else {
                self.set_run(&id, "queued", Some(&Value::Null))?;
                match self.contact_run_is_current(&id) {
                    Ok(_) => resume.push(id),
                    Err(e) => self.source_fail(&id, &e)?,
                }
            }
        }
        Ok(resume)
    }
}
