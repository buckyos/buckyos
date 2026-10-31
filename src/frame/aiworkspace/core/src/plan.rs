//! Commit planning (design doc §2.5–§2.7): turn one Commit request into a
//! candidate state layered over the current one, without touching storage.
//!
//! Planning runs inside the single writer, so `expect` checks are evaluated
//! here against the current state. All detectable conflicts of a batch are
//! collected; any hard error rejects the whole batch.

use crate::access::{own_note_block, own_note_payload, Access, Cap};
use crate::canonical::{canonical_json, sha256_hex};
use crate::error::{Code, WsError, WsResult};
use crate::id::{check_id, check_idempotency_key, check_name};
use crate::model::*;
use crate::order_key::check_order_key;
use crate::richtext;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_ops: usize,
    pub max_set_values: usize,
    pub max_migrate_rows: usize,
    pub max_inverse_bytes: usize,
    pub richtext: richtext::Limits,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_ops: 10_000,
            max_set_values: 50_000,
            max_migrate_rows: 100_000,
            max_inverse_bytes: 64 * 1024 * 1024,
            richtext: richtext::Limits::default(),
        }
    }
}

/// Everything non-deterministic is injected, so the backend, the WASM replica
/// and tests compute identical plans.
#[derive(Debug, Clone)]
pub struct PlanEnv {
    /// The `seq` this Commit will get if accepted.
    pub seq: u64,
    pub principal: String,
    pub origin: String,
    pub run_id: Option<String>,
    /// Acceptance time, `YYYY-MM-DDTHH:MM:SS.mmmZ`.
    pub now: String,
    /// Internal callers (undo, import) may use internal operations and `expect: "any"`.
    pub internal: bool,
    /// Loading a package: reference targets may be created later in the same
    /// batch (checked once at the end) and listed-missing assets are tolerated.
    pub import: bool,
    /// Admission runs the business Schema validators (D01); replay of an accepted Commit only checks the
    /// primitives' shape (`false`).
    pub schema_checks: bool,
    /// Re-applying an already accepted operation (a replica advancing its
    /// confirmed layer): CRDT bytes the backend produced are imported as they
    /// are instead of being re-generated.
    pub replay: bool,
    /// CRDT peer used for backend-authored rich text operations.
    pub peer_id: u64,
    pub limits: Limits,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitRequest {
    pub protocol_version: String,
    pub workspace_id: String,
    pub epoch: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default = "default_origin")]
    pub origin: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub undo_group: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub preconditions: Vec<Value>,
    pub operations: Vec<Value>,
}

fn default_origin() -> String {
    "human".into()
}

impl CommitRequest {
    pub fn parse(v: &Value) -> WsResult<CommitRequest> {
        let req: CommitRequest =
            serde_json::from_value(v.clone()).map_err(|e| WsError::invalid_schema(format!("bad commit request: {e}")))?;
        if !supports_protocol(&req.protocol_version) {
            return Err(WsError::new(Code::UnsupportedVersion, format!("protocol_version {} not supported", req.protocol_version)));
        }
        check_idempotency_key(&req.idempotency_key)?;
        if let Some(s) = &req.session_id {
            check_id("session_id", s)?;
        }
        if !matches!(req.origin.as_str(), "human" | "agent" | "program" | "import" | "system") {
            return Err(WsError::invalid_op("unknown origin"));
        }
        Ok(req)
    }

    /// `sha256(JCS(request without session_id and message))`
    pub fn digest(v: &Value) -> WsResult<String> {
        let mut o = v.as_object().cloned().unwrap_or_default();
        o.remove("session_id");
        o.remove("message");
        Ok(sha256_hex(canonical_json(&Value::Object(o))?.as_bytes()))
    }
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub op_index: usize,
    pub item_index: Option<usize>,
    pub code: Code,
    pub entity_id: String,
    pub selector: Option<Value>,
    pub expected: Value,
    pub current: Value,
    /// Only filled when the principal may read the target.
    pub current_value: Option<Value>,
    pub detail: Option<String>,
}

impl Conflict {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("op_index".into(), json!(self.op_index));
        if let Some(i) = self.item_index {
            m.insert("item_index".into(), json!(i));
        }
        m.insert("code".into(), json!(self.code.as_str()));
        m.insert("entity_id".into(), json!(self.entity_id));
        if let Some(s) = &self.selector {
            m.insert("selector".into(), s.clone());
        }
        m.insert("expected_rev".into(), self.expected.clone());
        m.insert("current_rev".into(), self.current.clone());
        if let Some(v) = &self.current_value {
            m.insert("current_value".into(), v.clone());
        }
        if let Some(d) = &self.detail {
            m.insert("detail".into(), json!(d));
        }
        Value::Object(m)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Touched {
    pub entity_id: String,
    pub selector: Option<Value>,
    /// created | deleted | restored | renamed | moved | placed | value | schema | view | text | derived
    pub change: &'static str,
    pub rev: u64,
}

impl Touched {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("entity_id".into(), json!(self.entity_id));
        if let Some(s) = &self.selector {
            m.insert("selector".into(), s.clone());
        }
        m.insert("change".into(), json!(self.change));
        m.insert("rev".into(), json!(self.rev));
        Value::Object(m)
    }
    /// Changes that make results derived from the entity's data stale (V19).
    pub fn invalidates_data(change: &str) -> bool {
        matches!(change, "value" | "schema" | "text" | "deleted")
    }
}

#[derive(Debug, Clone)]
pub struct PlannedOp {
    /// Normalized operation as stored in history and replayed by replicas
    /// (rich text ops carry the CRDT bytes the backend produced).
    pub op: Value,
    /// `None` = not compensable by the backend.
    pub inverse: Option<Vec<Value>>,
    pub touched: Vec<Touched>,
}

pub enum CommitFailure {
    Conflict(Vec<Conflict>),
    Rejected { error: WsError, op_index: Option<usize> },
}

impl CommitFailure {
    pub fn to_json(&self) -> Value {
        match self {
            CommitFailure::Conflict(c) => {
                let code = c.first().map(|x| x.code).unwrap_or(Code::RevisionConflict);
                json!({ "status": "conflict", "code": code.as_str(), "retryable": false,
                        "conflicts": c.iter().map(Conflict::to_json).collect::<Vec<_>>() })
            }
            CommitFailure::Rejected { error, op_index } => {
                let mut e = error.to_json();
                if let Some(i) = op_index {
                    e["op_index"] = json!(i);
                }
                let mut out = json!({ "status": "rejected", "code": error.code.as_str(),
                                      "retryable": error.code.retryable(), "errors": [e] });
                if let Some(s) = error.sub {
                    out["sub_code"] = json!(s);
                }
                out
            }
        }
    }
}

pub struct CommitPlan<'a> {
    pub overlay: Overlay<'a>,
    pub ops: Vec<PlannedOp>,
    /// Entities with `write_policy = lock_required` whose lock this Commit needs.
    pub lock_targets: BTreeSet<String>,
    /// Extra reports for `prepare` (migration pre-checks).
    pub reports: Vec<Value>,
    pub warnings: Vec<Value>,
    /// Record ids inserted without read permission on their table.
    pub read_denied: BTreeSet<String>,
}

pub struct Planner<'a> {
    pub ov: Overlay<'a>,
    pub env: &'a PlanEnv,
    pub access: &'a Access,
    pub conflicts: Vec<Conflict>,
    pub op_index: usize,
    pub touched: Vec<Touched>,
    pub lock_targets: BTreeSet<String>,
    pub reports: Vec<Value>,
    pub warnings: Vec<Value>,
    /// Records inserted by an append-only principal (the only ones it may see back).
    pub read_denied: BTreeSet<String>,
}

pub type OpResult = WsResult<(Value, Option<Vec<Value>>)>;

pub fn sel(v: Value) -> Option<Value> {
    Some(v)
}

pub fn selector_string(v: &Value) -> String {
    canonical_json(v).unwrap_or_default()
}

pub fn get_str<'v>(op: &'v Value, key: &str) -> WsResult<&'v str> {
    op.get(key).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{key} required")).at(format!("/{key}")))
}

pub fn get_array<'v>(op: &'v Value, key: &str) -> WsResult<&'v Vec<Value>> {
    op.get(key).and_then(Value::as_array).ok_or_else(|| WsError::invalid_op(format!("{key} must be an array")).at(format!("/{key}")))
}

pub fn only_keys(op: &Value, allowed: &[&str]) -> WsResult<()> {
    for k in op.as_object().map(|o| o.keys()).into_iter().flatten() {
        if k != "op" && !allowed.contains(&k.as_str()) {
            return Err(WsError::invalid_op(format!("unknown parameter {k}")).at(format!("/{k}")));
        }
    }
    Ok(())
}

impl<'a> Planner<'a> {
    pub fn new(base: &'a dyn ReadCtx, env: &'a PlanEnv, access: &'a Access) -> Self {
        Planner {
            ov: Overlay::new(base),
            env,
            access,
            conflicts: vec![],
            op_index: 0,
            touched: vec![],
            lock_targets: BTreeSet::new(),
            reports: vec![],
            warnings: vec![],
            read_denied: BTreeSet::new(),
        }
    }

    pub fn seq(&self) -> u64 {
        self.env.seq
    }

    /// The entity if it exists and is visible to the caller's scope.
    pub fn entity_opt(&self, id: &str) -> WsResult<Option<EntityRow>> {
        Ok(self.ov.entity(id)?.filter(|e| self.access.sees_scope(e)))
    }

    pub fn entity_alive(&self, id: &str) -> WsResult<EntityRow> {
        match self.entity_opt(id)? {
            None => Err(WsError::not_found(format!("entity {id} not found")).with_data(json!({ "entity_id": id }))),
            Some(e) if !e.alive() => {
                Err(WsError::deleted(format!("entity {id} is deleted")).with_data(json!({ "entity_id": id })))
            }
            Some(e) => Ok(e),
        }
    }

    pub fn require(&self, entity_id: &str, cap: Cap) -> WsResult<()> {
        self.access.require(&self.ov, entity_id, cap)
    }

    pub fn can(&self, entity_id: &str, cap: Cap) -> WsResult<bool> {
        self.access.can(&self.ov, entity_id, cap)
    }

    /// A write to `e` that needs `cap` on `on`, made by the author of `e` holding only `comment` there: its own
    /// note Block (and, with `annotations`, its own annotation) — see [`own_note_block`]. Such a write takes
    /// no lock. False when the caller holds `cap` or `e` is not its own writing: the usual checks apply.
    fn by_comment(&self, e: &EntityRow, on: &str, cap: Cap, annotations: bool) -> WsResult<bool> {
        if self.can(on, cap)? {
            return Ok(false);
        }
        let mine = if e.type_id == TYPE_ANNOTATION {
            annotations && crate::types::annotation_author(e) == Some(self.env.principal.as_str())
        } else {
            own_note_block(&self.ov, &self.env.principal, e)?
        };
        if mine {
            self.require(on, Cap::Comment)?;
        }
        Ok(mine)
    }

    /// Writing `e`'s content (or, for containers, its direct membership) needs
    /// its lock when the entity demands one.
    pub fn need_lock(&mut self, e: &EntityRow) {
        if e.write_policy == POLICY_LOCK {
            self.lock_targets.insert(e.entity_id.clone());
        }
    }

    pub fn need_parent_lock(&mut self, child_id: &str) -> WsResult<()> {
        if let Some(edge) = self.ov.edge(child_id)? {
            if let Some(p) = self.ov.entity(&edge.parent_id)? {
                self.need_lock(&p);
            }
        }
        Ok(())
    }

    pub fn touch(&mut self, entity_id: &str, selector: Option<Value>, change: &'static str) {
        let t = Touched { entity_id: entity_id.to_string(), selector, change, rev: self.env.seq };
        if !self.touched.contains(&t) {
            self.touched.push(t);
        }
    }

    pub fn conflict(
        &mut self,
        code: Code,
        item_index: Option<usize>,
        entity_id: &str,
        selector: Option<Value>,
        expected: Value,
        current: Value,
        current_value: Option<Value>,
    ) {
        let readable = self.can(entity_id, Cap::Read).unwrap_or(false);
        self.conflicts.push(Conflict {
            op_index: self.op_index,
            item_index,
            code,
            entity_id: entity_id.to_string(),
            selector,
            expected,
            current,
            current_value: current_value.filter(|_| readable),
            detail: None,
        });
    }

    /// Check a mandatory `expect` against a version cell. Returns `false` (and
    /// records a conflict) on mismatch. A cell written earlier in this same
    /// batch (`rev == seq`) always passes: the batch is one atomic intent.
    pub fn check_rev(
        &mut self,
        expect: Option<&Value>,
        current: u64,
        item_index: Option<usize>,
        entity_id: &str,
        selector: Option<Value>,
        current_value: impl FnOnce() -> Option<Value>,
    ) -> WsResult<bool> {
        let expect = expect.ok_or_else(|| WsError::invalid_op("expect required").at("/expect"))?;
        self.check_rev_opt(Some(expect), current, item_index, entity_id, selector, current_value)
    }

    /// Like [`check_rev`] but a missing `expect` means "no check" (auto-merge operations).
    pub fn check_rev_opt(
        &mut self,
        expect: Option<&Value>,
        current: u64,
        item_index: Option<usize>,
        entity_id: &str,
        selector: Option<Value>,
        current_value: impl FnOnce() -> Option<Value>,
    ) -> WsResult<bool> {
        let Some(expect) = expect else { return Ok(true) };
        if expect == &json!("any") {
            return if self.env.internal {
                Ok(true)
            } else {
                Err(WsError::invalid_op("expect: \"any\" is not available to this caller").at("/expect"))
            };
        }
        let want = expect
            .as_object()
            .filter(|o| o.len() == 1)
            .and_then(|o| o.get("rev"))
            .and_then(Value::as_u64)
            .ok_or_else(|| WsError::invalid_op("expect must be { \"rev\": N }").at("/expect"))?;
        if want == current || current == self.env.seq {
            return Ok(true);
        }
        let cv = current_value();
        self.conflict(Code::RevisionConflict, item_index, entity_id, selector, json!(want), json!(current), cv);
        Ok(false)
    }

    pub fn bump_content(&self, e: &mut EntityRow) {
        e.content_rev = self.env.seq;
    }

    /// Alive siblings under `parent` whose name equals `name` (excluding `except`).
    pub fn name_taken(&self, parent: &str, name: &str, except: &str) -> WsResult<bool> {
        for edge in self.ov.children(parent)? {
            if edge.child_id == except {
                continue;
            }
            if let Some(e) = self.ov.entity(&edge.child_id)? {
                if e.alive() && e.scope == SCOPE_SHARED && e.name.as_deref() == Some(name) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// A local reference target must exist, be alive, be readable by the caller
    /// and (optionally) have one of the given types.
    pub fn check_ref_target(&self, reference: &Value, types: Option<&[&str]>) -> WsResult<Option<EntityRow>> {
        if !crate::value::reference_is_local(reference) {
            return Ok(None); // cross-Workspace targets are authorized when resolved, not here
        }
        if reference["version"]["mode"] == json!("published_channel") {
            return Ok(None);
        }
        let id = crate::value::reference_entity_id(reference).unwrap_or("");
        if self.env.import && self.ov.entity(id)?.is_none() {
            return Ok(None); // verified by `check_deferred_refs` once the whole package is staged
        }
        let target = match self.entity_opt(id)? {
            Some(t) if t.alive() && self.access.can_read(&self.ov, &t)? => t,
            // unreadable targets are indistinguishable from missing ones
            _ => {
                return Err(WsError::new(Code::ReferenceBroken, format!("reference target {id} not found"))
                    .with_data(json!({ "entity_id": id })))
            }
        };
        if let Some(types) = types {
            if !types.contains(&target.type_id.as_str()) {
                return Err(WsError::invalid_schema(format!("reference target {id} has type {}", target.type_id)));
            }
        }
        Ok(Some(target))
    }
}

/// The version cell (or content token) a selector names — used by Commit-level
/// read-set preconditions.
/// The selector kinds `resolve_cell` knows (D04, frozen; `schemas/commit-request.schema.json` lists the same).
pub const SELECTOR_KINDS: &[&str] = &[
    "entity", "doc_key", "table_members", "table_field", "table_field_type", "table_field_values", "table_record", "table_cell",
    "tree_children", "tree_edge", "richtext_block", "folder_member", "table_view", "table_query",
];

pub fn resolve_cell(ctx: &dyn ReadCtx, target: &Value) -> WsResult<Value> {
    let entity_id = get_str(target, "entity_id")?;
    let e = ctx.entity(entity_id)?.ok_or_else(|| WsError::not_found(format!("entity {entity_id} not found")))?;
    if !e.alive() {
        return Err(WsError::deleted(format!("entity {entity_id} is deleted")));
    }
    // both `{entity_id, selector:{kind,..}}` and the flat `{entity_id, record_id, field_id}` form are accepted
    let flat = target.get("selector").is_none();
    let s = if flat { target } else { &target["selector"] };
    let kind = match s.get("kind").and_then(Value::as_str) {
        Some(k) => k,
        None if s.get("record_id").is_some() && s.get("field_id").is_some() => "table_cell",
        None if s.get("record_id").is_some() => "table_record",
        None if s.get("field_id").is_some() => "table_field",
        None if s.get("key").is_some() => "doc_key",
        None if s.get("block_id").is_some() => "richtext_block",
        None => "entity",
    };
    let field = |need_alive: bool| -> WsResult<FieldRow> {
        let f = get_str(s, "field_id")?;
        match ctx.field(entity_id, f)? {
            Some(row) if row.alive() || !need_alive => Ok(row),
            Some(_) => Err(WsError::deleted(format!("field {f} is deleted"))),
            None => Err(WsError::not_found(format!("field {f} not found"))),
        }
    };
    let record = || -> WsResult<RecordRow> {
        let r = get_str(s, "record_id")?;
        match ctx.record(entity_id, r)? {
            Some(row) if row.alive() => Ok(row),
            Some(_) => Err(WsError::deleted(format!("record {r} is deleted"))),
            None => Err(WsError::not_found(format!("record {r} not found"))),
        }
    };
    Ok(match kind {
        "entity" => json!(e.content_rev),
        "doc_key" => json!(e.key_rev(get_str(s, "key")?)),
        "table_members" => json!(e.key_rev(MEMBERS_KEY)),
        "table_field" => json!(field(true)?.def_rev),
        "table_field_type" => json!(field(true)?.type_rev),
        "table_field_values" => json!(field(true)?.values_rev),
        "table_record" => json!(record()?.rev),
        "table_cell" => {
            let f = field(true)?;
            json!(record()?.value_rev(&f.field_id))
        }
        // the identities of the live direct children (a folder input: a new member makes it stale)
        "tree_children" => {
            let mut ids: Vec<String> = Vec::new();
            for edge in ctx.children(entity_id)? {
                if ctx.entity(&edge.child_id)?.is_some_and(|c| c.alive()) {
                    ids.push(edge.child_id);
                }
            }
            ids.sort();
            json!(crate::canonical::sha256_hex(ids.join("\n").as_bytes())[..32].to_string())
        }
        // where the node sits (parent, order, placement)
        "tree_edge" => json!(ctx.edge(entity_id)?.map_or(0, |e| e.struct_rev)),
        "richtext_block" => {
            let b = get_str(s, "block_id")?;
            let rt = ctx.richtext(entity_id)?.ok_or_else(|| WsError::invalid_op("not a rich text"))?;
            match rt.meta.block_index.get(b) {
                Some(info) => json!(info.hash),
                None => return Err(WsError::deleted(format!("block {b} not found"))),
            }
        }
        // one member of a resource directory (D01): its own version cell; a removed member is gone
        "folder_member" => {
            let name = get_str(s, "name")?;
            if e.type_id != TYPE_DIR {
                return Err(WsError::invalid_op(format!("{entity_id} is not a directory")));
            }
            if e.payload.get("members").and_then(|m| m.get(name)).is_none() {
                return Err(WsError::deleted(format!("member {name} is not in {entity_id}")));
            }
            json!(e.key_rev(&format!("m:{name}")))
        }
        // a saved table view (`entity_id` = the table, `cell_id` = the view Block): the view's configuration
        // keys plus the table's membership and every live field's type and values — the coarse token a wish
        // input declares (D04); the read set still records the fine cells
        "table_view" => {
            let cell_id = get_str(s, "cell_id")?;
            let c = ctx.entity(cell_id)?.filter(|c| c.alive()).ok_or_else(|| WsError::deleted(format!("view {cell_id} not found")))?;
            if c.type_id != TYPE_CELL
                || c.payload.get("view").and_then(|v| v["type"].as_str()) != Some("table")
                || c.payload.get("source_ref").and_then(crate::value::reference_entity_id) != Some(entity_id)
            {
                return Err(WsError::invalid_op(format!("{cell_id} is not a table view of {entity_id}")));
            }
            let mut parts: Vec<String> = ["filter", "sorts", "group", "fields", "manual_order", "source_ref"].iter().map(|k| format!("{k}={}", c.key_rev(k))).collect();
            table_token(ctx, &e, None, &mut parts)?;
            token(&parts)
        }
        // a table query (`filter? / sorts? / fields?`): the table's membership plus the type and values of the
        // fields it projects (every live field without `fields`)
        "table_query" => {
            let fields: Option<Vec<String>> = s.get("fields").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            let mut parts = Vec::new();
            table_token(ctx, &e, fields.as_deref(), &mut parts)?;
            token(&parts)
        }
        _ => return Err(WsError::new(Code::MissingExtension, format!("unknown selector kind {kind}"))),
    })
}

fn token(parts: &[String]) -> Value {
    json!(sha256_hex(parts.join("\n").as_bytes())[..32].to_string())
}

/// The version parts of a table's rows as a selector sees them: membership, then each field's type and values.
fn table_token(ctx: &dyn ReadCtx, table: &EntityRow, fields: Option<&[String]>, parts: &mut Vec<String>) -> WsResult<()> {
    if table.type_id != TYPE_TABLE {
        return Err(WsError::invalid_op(format!("{} is not a table", table.entity_id)));
    }
    parts.push(format!("members={}", table.key_rev(MEMBERS_KEY)));
    let live: Vec<FieldRow> = match fields {
        Some(ids) => {
            let mut rows = Vec::new();
            for f in ids {
                match ctx.field(&table.entity_id, f)? {
                    Some(row) if row.alive() => rows.push(row),
                    Some(_) => return Err(WsError::deleted(format!("field {f} is deleted"))),
                    None => return Err(WsError::not_found(format!("field {f} not found"))),
                }
            }
            rows
        }
        None => crate::filter::live_fields(ctx, &table.entity_id)?,
    };
    for f in live {
        parts.push(format!("{}={}/{}", f.field_id, f.type_rev, f.values_rev));
    }
    Ok(())
}

fn check_preconditions(p: &mut Planner, pre: &[Value]) -> WsResult<()> {
    for (i, c) in pre.iter().enumerate() {
        let target = c.get("target").ok_or_else(|| WsError::invalid_op("precondition needs target"))?;
        let expect = c.get("expect").ok_or_else(|| WsError::invalid_op("precondition needs expect"))?;
        let entity_id = get_str(target, "entity_id")?.to_string();
        let want = expect.get("rev").or_else(|| expect.get("hash")).cloned().ok_or_else(|| {
            WsError::invalid_op("precondition expect must be { rev } or { hash }")
        })?;
        let (code, current) = match resolve_cell(&p.ov, target) {
            Ok(cur) => (Code::RevisionConflict, cur),
            Err(e) if e.code == Code::TargetDeleted || e.code == Code::NotFound => (Code::TargetDeleted, Value::Null),
            Err(e) => return Err(e),
        };
        if current != want {
            p.op_index = usize::MAX;
            let selector = target.get("selector").cloned().or_else(|| Some(target.clone()));
            p.conflict(code, Some(i), &entity_id, selector, want, current, None);
            if let Some(last) = p.conflicts.last_mut() {
                last.detail = Some("read-set precondition failed".into());
            }
        }
    }
    Ok(())
}

/// Version of the planner's normalization (内核架构改进 D20): accepted Commits are applied from their
/// recorded row delta, so a replica never needs this to match; `ws.get_info` reports it for diagnostics.
pub const PLANNER_VERSION: u32 = 1;

/// The operation catalogue: `(name, class, undo, capability)` — part of the
/// protocol (design doc §2.5.3). `schemas/operations.json` mirrors it.
pub const OPERATIONS: &[(&str, &str, &str, &str)] = &[
    ("entity.create", "create", "compensable", "structure"),
    ("entity.delete", "structural", "compensable", "delete"),
    ("entity.restore", "structural", "compensable", "delete"),
    ("entity.rename", "overwrite", "compensable", "structure"),
    ("entity.set_keys", "overwrite", "compensable", "update"),
    ("entity.unset_keys", "overwrite", "compensable", "update"),
    ("entity.set_write_policy", "overwrite", "compensable", "manage"),
    ("entity.set_derived", "merge", "compensable", "update"),
    ("tree.move", "merge", "compensable", "structure"),
    ("tree.place", "merge", "compensable", "structure"),
    ("tree.clone", "create", "compensable", "structure"),
    ("entity.clone", "create", "compensable", "structure"),
    ("data.pin", "merge", "compensable", "structure"),
    ("dir.set_member", "overwrite", "compensable", "update"),
    ("dir.remove_member", "overwrite", "compensable", "update"),
    ("dir.replace", "overwrite", "compensable", "update"),
    ("table.insert_records", "append", "compensable", "append"),
    ("table.delete_records", "structural", "compensable", "delete"),
    ("table.set_values", "overwrite", "compensable", "update"),
    ("table.unset_values", "overwrite", "compensable", "update"),
    ("table.set_body", "overwrite", "compensable", "update"),
    ("table.add_field", "structural", "compensable", "structure"),
    ("table.update_field", "structural", "compensable", "structure"),
    ("table.delete_field", "structural", "compensable", "structure"),
    ("table.add_option", "structural", "compensable", "structure"),
    ("table.update_option", "structural", "compensable", "structure"),
    ("table.delete_option", "structural", "compensable", "structure"),
    ("table.migrate_field", "structural", "compensable", "structure"),
    ("richtext.apply_update", "merge", "session_local", "update"),
    ("richtext.insert_blocks", "overwrite", "compensable", "update"),
    ("richtext.replace_block", "overwrite", "compensable", "update"),
    ("richtext.delete_blocks", "overwrite", "compensable", "update"),
    ("richtext.move_block", "overwrite", "compensable", "update"),
    // internal: produced by the undo path only
    ("table.restore_records", "internal", "compensable", "delete"),
    ("table.restore_field", "internal", "compensable", "structure"),
    ("table.restore_migration", "internal", "compensable", "structure"),
    ("data.restore_owns", "internal", "compensable", "structure"),
    ("entity.purge", "internal", "none", "manage"),
];

/// Operations only the undo path / import may submit.
fn is_internal_op(name: &str) -> bool {
    matches!(name, "table.restore_records" | "table.restore_field" | "table.restore_migration" | "data.restore_owns" | "entity.purge")
}

/// Marker a purged entity keeps in its (otherwise emptied) payload.
pub const PURGED_KEY: &str = "#purged";

/// `entity.purge { entity_id }` (internal, the maintenance channel D19): a deleted entity gives up its
/// content and references for good — its resources become reclaimable at once and it can never be
/// restored (`RESOURCE_RECLAIMED`). Not compensable.
fn entity_purge(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id"])?;
    let id = get_str(op, "entity_id")?;
    let mut e = p.entity_opt(id)?.ok_or_else(|| WsError::not_found(format!("entity {id} not found")))?;
    if e.alive() {
        return Err(WsError::invalid_op(format!("entity {id} is not deleted")));
    }
    let seq = p.seq();
    let old = crate::types::entity_refs(&e)?;
    p.ov.set_refs(&old, &BTreeSet::new());
    for r in p.ov.refs_from(id)? {
        p.ov.del_ref(r);
    }
    e.payload = Map::new();
    e.payload.insert(PURGED_KEY.into(), json!(seq));
    e.key_revs = BTreeMap::new();
    e.derived = None;
    e.content_rev = seq;
    p.ov.put_entity(e);
    p.touch(id, None, "purged");
    Ok((op.clone(), None))
}

/// Own data (内核架构改进 D05): a Block created in this Commit together with the data it shows, under the
/// canvas content folder, owns that data — unless the Block declares `data_ownership: "shared"`. The
/// `owns` edge is added beside `bind`; data of the data tree (inserted in data mode) is never owned.
fn add_owns_edges(p: &mut Planner) -> WsResult<()> {
    let seq = p.seq();
    let created: Vec<EntityRow> = p.ov.entities.values().filter(|e| e.created_seq == seq && e.alive() && e.type_id == TYPE_CELL).cloned().collect();
    for cell in created {
        if cell.payload.get("data_ownership").and_then(Value::as_str) == Some("shared") {
            continue;
        }
        let Some(target) = cell.payload.get("source_ref").filter(|r| crate::value::reference_is_local(r)).and_then(crate::value::reference_entity_id).map(str::to_string) else { continue };
        let Some(data) = p.ov.entity(&target)? else { continue };
        if data.created_seq != seq || !data.alive() || !under_canvas_content(&p.ov, &target)? {
            continue;
        }
        p.ov.add_ref(RefEdge::local(&cell.entity_id, "", "owns", &target));
    }
    Ok(())
}

fn under_canvas_content(ctx: &dyn ReadCtx, id: &str) -> WsResult<bool> {
    let mut cur = id.to_string();
    for _ in 0..4096 {
        match ctx.edge(&cur)? {
            Some(e) if e.parent_id == CANVAS_CONTENT_ID => return Ok(true),
            Some(e) => cur = e.parent_id,
            None => return Ok(false),
        }
    }
    Ok(false)
}

/// Own data nobody shows any more goes with the last Block that showed it (D05): the data the
/// `doomed` entities bind that was ever owned (an `owns` edge from any Block, deleted ones included —
/// the index keeps them) and keeps no `bind` from an alive entity outside `doomed`.
fn orphaned_own_data(p: &Planner, doomed: &BTreeSet<String>) -> WsResult<Vec<String>> {
    let mut out = Vec::new();
    for src in doomed {
        for r in p.ov.refs_from(src)? {
            if !matches!(r.kind.as_str(), "bind" | "owns") || !r.dst_workspace_id.is_empty() || r.dst_entity_id.is_empty() || out.contains(&r.dst_entity_id) || doomed.contains(&r.dst_entity_id) {
                continue;
            }
            let Some(data) = p.ov.entity(&r.dst_entity_id)? else { continue };
            if !data.alive() {
                continue;
            }
            let (mut owned, mut shown) = (false, false);
            for b in p.ov.refs_to(&r.dst_entity_id)? {
                if b.kind == "owns" {
                    owned = true;
                } else if b.kind == "bind" && !doomed.contains(&b.src_entity_id) && p.ov.entity(&b.src_entity_id)?.is_some_and(|s| s.alive()) {
                    shown = true;
                }
            }
            if owned && !shown {
                out.push(r.dst_entity_id.clone());
            }
        }
    }
    Ok(out)
}

/// `data.pin { entity_id }`: the data stops being any Block's own (its `owns` edges go); it is shared
/// data of the data tree from now on and never follows a Block's deletion.
fn data_pin(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let e = p.entity_alive(id)?;
    p.require(id, Cap::Structure)?;
    if !p.check_rev_opt(op.get("expect"), e.meta_rev, None, id, None, || None)? {
        return Ok((op.clone(), None));
    }
    let owners: Vec<String> = p.ov.refs_to(id)?.into_iter().filter(|r| r.kind == "owns").map(|r| r.src_entity_id).collect();
    for o in &owners {
        p.ov.del_ref(RefEdge::local(o, "", "owns", id));
    }
    p.touch(id, None, "pinned");
    let inverse = json!({ "op": "data.restore_owns", "entity_id": id, "owners": owners });
    Ok((op.clone(), Some(vec![inverse])))
}

fn data_restore_owns(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "owners"])?;
    let id = get_str(op, "entity_id")?;
    p.entity_alive(id)?;
    let owners: Vec<String> = op.get("owners").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    for o in &owners {
        if p.ov.entity(o)?.is_some_and(|s| s.alive()) {
            p.ov.add_ref(RefEdge::local(o, "", "owns", id));
        }
    }
    p.touch(id, None, "pinned");
    Ok((op.clone(), Some(vec![json!({ "op": "data.pin", "entity_id": id })])))
}

fn member_sel(name: &str) -> Option<Value> {
    Some(json!({ "kind": "folder_member", "name": name }))
}

/// The directory an operation writes: alive, a `buckyos.dir`, `update` on it, locked by this session when
/// its policy asks for it. Returns the row and its member map.
fn dir_target(p: &mut Planner, id: &str) -> WsResult<(EntityRow, Map<String, Value>)> {
    let e = p.entity_alive(id)?;
    if e.type_id != TYPE_DIR {
        return Err(WsError::invalid_op(format!("{id} is not a directory")));
    }
    if let Some(d) = e.degraded() {
        return Err(WsError::new(Code::UnsupportedVersion, format!("directory {id} cannot be edited ({d})")));
    }
    p.require(id, Cap::Update)?;
    crate::types::require_source_writable(p, &e)?;
    p.need_lock(&e);
    let members = e.payload.get("members").and_then(Value::as_object).cloned().unwrap_or_default();
    Ok((e, members))
}

/// Write the member map back: validation, references, revisions, touch.
fn dir_write(p: &mut Planner, before: &EntityRow, mut e: EntityRow, members: Map<String, Value>, changed: &[String]) -> WsResult<()> {
    let seq = p.seq();
    e.payload.insert("members".into(), Value::Object(members));
    crate::types::validate_dir(p, &mut e)?;
    for name in changed {
        e.key_revs.insert(format!("m:{name}"), seq);
        p.touch(&e.entity_id, member_sel(name), "value");
    }
    e.key_revs.insert("members".into(), seq);
    e.content_rev = seq;
    let old = crate::types::entity_refs(before)?;
    let new = crate::types::entity_refs(&e)?;
    p.ov.set_refs(&old, &new);
    p.ov.put_entity(e);
    Ok(())
}

/// `dir.set_member { entity_id, name, member, expect }`: add or replace one member; `expect` is the member's
/// cell (`{ rev: 0 }` for a new name).
fn dir_set_member(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "name", "member", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let name = get_str(op, "name")?;
    crate::types::check_member_name(name)?;
    let (before, mut members) = dir_target(p, id)?;
    let old = members.get(name).cloned();
    if !p.check_rev(op.get("expect"), before.key_rev(&format!("m:{name}")), None, id, member_sel(name), || old.clone())? {
        return Ok((op.clone(), None));
    }
    let member = op.get("member").cloned().filter(Value::is_object).ok_or_else(|| WsError::invalid_schema("member must be an object").at("/member"))?;
    members.insert(name.to_string(), member);
    let seq = p.seq();
    dir_write(p, &before, before.clone(), members, &[name.to_string()])?;
    let inverse = match old {
        Some(m) => json!({ "op": "dir.set_member", "entity_id": id, "name": name, "member": m, "expect": { "rev": seq } }),
        None => json!({ "op": "dir.remove_member", "entity_id": id, "name": name, "expect": { "rev": seq } }),
    };
    Ok((op.clone(), Some(vec![inverse])))
}

/// `dir.remove_member { entity_id, name, expect }`.
fn dir_remove_member(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "name", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let name = get_str(op, "name")?;
    let (before, mut members) = dir_target(p, id)?;
    let Some(old) = members.remove(name) else { return Err(WsError::not_found(format!("member {name} is not in {id}"))) };
    if !p.check_rev(op.get("expect"), before.key_rev(&format!("m:{name}")), None, id, member_sel(name), || Some(old.clone()))? {
        return Ok((op.clone(), None));
    }
    let seq = p.seq();
    dir_write(p, &before, before.clone(), members, &[name.to_string()])?;
    let inverse = json!({ "op": "dir.set_member", "entity_id": id, "name": name, "member": old, "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inverse])))
}

/// `dir.replace { entity_id, members, expect }`: the whole map at once (a switch to another DirObject);
/// `expect` is the directory's `content_rev`.
fn dir_replace(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "members", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let (before, old) = dir_target(p, id)?;
    if !p.check_rev(op.get("expect"), before.content_rev, None, id, None, || Some(Value::Object(old.clone())))? {
        return Ok((op.clone(), None));
    }
    let members = op.get("members").and_then(Value::as_object).cloned().ok_or_else(|| WsError::invalid_schema("members must be an object").at("/members"))?;
    let mut changed: Vec<String> = old.keys().chain(members.keys()).cloned().collect();
    changed.sort();
    changed.dedup();
    let seq = p.seq();
    dir_write(p, &before, before.clone(), members, &changed)?;
    let inverse = json!({ "op": "dir.replace", "entity_id": id, "members": Value::Object(old), "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inverse])))
}

/// `entity.clone { entity_id, new_entity_id, parent_id, order_key, name?, placement? }`: a deep copy of a
/// data entity (payload, table fields and live records, rich text content as a new lineage) — the one
/// place copy semantics live (D05): the Desktop and Agents clone own data through it. Record bodies and
/// children are not copied; generation records are not carried over.
fn entity_clone(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "new_entity_id", "parent_id", "order_key", "name", "placement"])?;
    let id = get_str(op, "entity_id")?;
    let new_id = get_str(op, "new_entity_id")?;
    let src = p.entity_alive(id)?;
    if !p.access.can_read(&p.ov, &src)? {
        return Err(WsError::not_found(format!("entity {id} not found")));
    }
    if is_system_id(id) || src.type_id == TYPE_CONTAINER || src.type_id == TYPE_CELL {
        return Err(WsError::invalid_op("entity.clone copies data entities; Blocks and containers are copied by creating them"));
    }
    let mut payload = src.payload.clone();
    let payload = match src.type_id.as_str() {
        TYPE_RECORD => crate::types::record_nested(&payload),
        TYPE_RICHTEXT => {
            let rt = p.ov.richtext(id)?.ok_or_else(|| WsError::invalid_op("rich text state missing"))?;
            json!({ "content": rt.meta.ast })
        }
        TYPE_TABLE => {
            payload.remove("source_ref");
            let fields: Vec<Value> = p.ov.fields(id)?.into_iter().filter(|f| f.alive()).map(|f| {
                let mut v = f.def.to_value();
                v["order_key"] = json!(f.order_key);
                v
            }).collect();
            payload.insert("fields".into(), Value::Array(fields));
            Value::Object(payload)
        }
        TYPE_WISH => {
            for k in ["last_run", "analysis", "program"] {
                payload.remove(k);
            }
            Value::Object(payload)
        }
        TYPE_ANNOTATION => {
            payload.remove("author");
            Value::Object(payload)
        }
        _ => Value::Object(payload),
    };
    let mut create = json!({ "op": "entity.create", "entity_id": new_id, "type_id": src.type_id, "schema_version": src.schema_version,
                             "parent_id": op["parent_id"], "order_key": op["order_key"], "payload": payload });
    for k in ["name", "placement"] {
        if let Some(v) = op.get(k) {
            create[k] = v.clone();
        }
    }
    let (_, inverse) = entity_create(p, &create)?;
    if src.type_id == TYPE_TABLE {
        let mut records = Vec::new();
        p.ov.scan_records(id, &mut |r| {
            if r.alive() {
                let mut rec = json!({ "record_id": r.record_id, "values": r.values });
                if !r.meta.is_empty() {
                    rec["meta"] = Value::Object(r.meta.clone());
                }
                records.push(rec);
            }
            Ok(true)
        })?;
        if !records.is_empty() {
            crate::plan_table::dispatch(p, "table.insert_records", &json!({ "op": "table.insert_records", "source_id": new_id, "records": records }))?;
        }
    }
    let mut norm = op.clone();
    norm["cloned_from_rev"] = json!(src.content_rev);
    Ok((norm, inverse))
}

/// The id a copy of `old` gets in a `tree.clone` seeded with `seed`: the same request plans the same ids everywhere.
pub fn clone_id(seed: &str, old: &str) -> String {
    format!("c{}", &sha256_hex(format!("{seed}\n{old}").as_bytes())[..26])
}

/// `tree.clone { source_id, parent_id, order_key?, placement?, slot?, content?: "private", id_seed? }` (扩展控件 P1,
/// Q17): a deep copy of a Block subtree — a Cell, a group or a control with everything under it — under `parent_id`,
/// in one operation the service, the WASM replica and an offline client plan alike. Every copy gets the id
/// `clone_id(id_seed, old id)` (`id_seed` defaults to `<seq>.<op index>`; a client passes its own to know the ids
/// beforehand); parents, order and placements carry over, `slot`s too, and the root takes `order_key` (default: last),
/// `placement` and `slot` (`null` drops it) when given. `def_ref` is kept: instances share their definition. Data a
/// copied Block owns (`owns`, `content: "private"`) is cloned with `entity.clone` into the target Surface's content
/// folder and the copy's `source_ref` / `bindings` follow it (the copy owns the clone); data bound from elsewhere stays
/// shared. A connector inside the subtree is copied when each bound end is in the subtree (rebound to the copies) and
/// left out when one crosses its border. Rights: `structure` on the target, `read` on the source. The normalized
/// operation records `id_seed` and `new_ids { <old>: <new> }` (data clones included); the Commit result lists them.
fn tree_clone(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "parent_id", "order_key", "placement", "slot", "content", "id_seed", "new_ids"])?;
    let source_id = get_str(op, "source_id")?;
    let parent_id = get_str(op, "parent_id")?;
    if op.get("content").is_some_and(|c| c != "private") {
        return Err(WsError::invalid_op("content must be \"private\" (owned data is cloned, shared data stays shared)"));
    }
    let seed = match op.get("id_seed") {
        None | Some(Value::Null) => format!("{}.{}", p.env.seq, p.op_index),
        Some(Value::String(s)) if !s.is_empty() && s.len() <= 64 => s.clone(),
        Some(_) => return Err(WsError::invalid_schema("id_seed must be a string of 1…64 bytes").at("/id_seed")),
    };
    let src = p.entity_alive(source_id)?;
    if !p.access.can_read(&p.ov, &src)? {
        return Err(WsError::not_found(format!("entity {source_id} not found")));
    }
    if is_system_id(source_id) || !(src.type_id == TYPE_CELL || crate::types::is_block_container(&src)) {
        return Err(WsError::invalid_op("tree.clone copies a Block, a group or a control with its subtree"));
    }
    // the subtree, parents first; another person's personal entities are not the caller's to copy
    let mut rows = vec![src.clone()];
    alive_descendants(p, source_id, &mut rows)?;
    rows.retain(|r| p.access.sees_scope(r));
    let inside: BTreeSet<String> = rows.iter().map(|r| r.entity_id.clone()).collect();
    let crosses = |r: &EntityRow| {
        crate::types::is_connector(&r.type_id, &r.payload)
            && ["start", "end"].iter().any(|k| r.payload.get(*k).and_then(|b| b.get("entity_id")).and_then(Value::as_str).is_some_and(|id| !inside.contains(id)))
    };
    rows.retain(|r| !crosses(r));
    let mut ids: BTreeMap<String, String> = rows.iter().map(|r| (r.entity_id.clone(), clone_id(&seed, &r.entity_id))).collect();
    // own data of the copied Blocks: cloned next to the target Surface's other own data
    let mut owned: Vec<String> = Vec::new();
    for r in &rows {
        for e in p.ov.refs_from(&r.entity_id)? {
            if e.kind == "owns" && e.dst_workspace_id.is_empty() && !owned.contains(&e.dst_entity_id) && p.ov.entity(&e.dst_entity_id)?.is_some_and(|d| d.alive()) {
                owned.push(e.dst_entity_id.clone());
            }
        }
    }
    let mut inverse_data = Vec::new();
    if !owned.is_empty() {
        let folder = match surface_of(&p.ov, parent_id)?.and_then(|s| s.payload.get("content_folder_id").and_then(Value::as_str).map(str::to_string)) {
            Some(f) => f,
            None => p.ov.edge(&owned[0])?.map(|e| e.parent_id).ok_or_else(|| WsError::invalid_op("owned data has no folder"))?,
        };
        let mut last = p.ov.children(&folder)?.last().map(|e| e.order_key.clone());
        for data_id in &owned {
            let new_id = clone_id(&seed, data_id);
            let key = crate::order_key::order_key_between(last.as_deref(), None)?;
            last = Some(key.clone());
            let (_, inv) = entity_clone(p, &json!({ "op": "entity.clone", "entity_id": data_id, "new_entity_id": new_id, "parent_id": folder, "order_key": key }))?;
            inverse_data.extend(inv.unwrap_or_default());
            ids.insert(data_id.clone(), new_id);
        }
    }
    let remap = |v: &mut Value, ids: &BTreeMap<String, String>| {
        if let Some(n) = v.get("entity_id").and_then(Value::as_str).and_then(|id| ids.get(id)) {
            v["entity_id"] = json!(n);
        }
    };
    let mut inverse_blocks = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let edge = p.ov.edge(&r.entity_id)?.ok_or_else(|| WsError::invalid_op("entity has no tree edge"))?;
        let mut payload = r.payload.clone();
        for key in ["source_ref", "start", "end"] {
            if let Some(v) = payload.get_mut(key).filter(|v| v.is_object()) {
                remap(v, &ids);
            }
        }
        if let Some(b) = payload.get_mut("bindings").and_then(Value::as_object_mut) {
            for v in b.values_mut() {
                remap(v, &ids);
            }
        }
        let mut create = json!({ "op": "entity.create", "entity_id": ids[&r.entity_id], "type_id": r.type_id, "schema_version": r.schema_version });
        if i == 0 {
            create["parent_id"] = json!(parent_id);
            create["order_key"] = match op.get("order_key").filter(|v| !v.is_null()) {
                Some(k) => k.clone(),
                None => json!(crate::order_key::order_key_between(p.ov.children(parent_id)?.last().map(|e| e.order_key.as_str()), None)?),
            };
            if let Some(pl) = op.get("placement").filter(|v| !v.is_null()).cloned().or(edge.placement.clone()) {
                create["placement"] = pl;
            }
            match op.get("slot") {
                Some(Value::Null) => {
                    payload.remove("slot");
                }
                Some(s) => {
                    payload.insert("slot".into(), s.clone());
                }
                None => {}
            }
            if let Some(n) = r.name.as_deref().filter(|n| r.scope == SCOPE_SHARED && !p.name_taken(parent_id, n, "").unwrap_or(true)) {
                create["name"] = json!(n);
            }
        } else {
            create["parent_id"] = json!(ids.get(&edge.parent_id).ok_or_else(|| WsError::invalid_op("subtree parent missing"))?);
            create["order_key"] = json!(edge.order_key);
            if let Some(pl) = &edge.placement {
                create["placement"] = pl.clone();
            }
            if let Some(n) = &r.name {
                create["name"] = json!(n);
            }
        }
        if let Some(sid) = &r.schema_id {
            create["schema_id"] = json!(sid);
        }
        create["payload"] = Value::Object(payload);
        let (_, inv) = entity_create(p, &create)?;
        inverse_blocks.extend(inv.unwrap_or_default());
    }
    inverse_blocks.reverse();
    inverse_blocks.extend(inverse_data);
    let mut norm = op.clone();
    norm["id_seed"] = json!(seed);
    norm["new_ids"] = json!(ids);
    Ok((norm, Some(inverse_blocks)))
}

/// Plan a whole Commit. On success the returned overlay is the candidate state.
pub fn plan_commit<'a>(
    base: &'a dyn ReadCtx,
    req: &CommitRequest,
    env: &'a PlanEnv,
    access: &'a Access,
) -> Result<CommitPlan<'a>, CommitFailure> {
    let reject = |error: WsError, op_index: Option<usize>| CommitFailure::Rejected { error, op_index };
    if req.operations.is_empty() {
        return Err(reject(WsError::invalid_op("a commit needs at least one operation"), None));
    }
    if req.operations.len() > env.limits.max_ops {
        return Err(reject(WsError::limit(format!("more than {} operations", env.limits.max_ops)), None));
    }
    let mut p = Planner::new(base, env, access);
    check_preconditions(&mut p, &req.preconditions).map_err(|e| reject(e, None))?;
    let mut ops = Vec::with_capacity(req.operations.len());
    for (i, op) in req.operations.iter().enumerate() {
        p.op_index = i;
        p.touched = vec![];
        let name = op.get("op").and_then(Value::as_str).unwrap_or("");
        let result = if is_internal_op(name) && !env.internal {
            Err(WsError::invalid_op(format!("{name} is an internal operation")))
        } else {
            dispatch(&mut p, name, op)
        };
        match result {
            Ok((op, inverse)) => ops.push(PlannedOp { op, inverse, touched: std::mem::take(&mut p.touched) }),
            Err(e) if e.code.is_conflict() => {
                // a competing change (deleted target, stale schema): report with the collected conflicts
                let entity_id = e.data.as_ref().and_then(|d| d["entity_id"].as_str()).unwrap_or("").to_string();
                p.conflicts.push(Conflict {
                    op_index: i,
                    item_index: None,
                    code: e.code,
                    entity_id,
                    selector: e.data.as_ref().and_then(|d| d.get("selector").cloned()),
                    expected: Value::Null,
                    current: Value::Null,
                    current_value: None,
                    detail: Some(e.detail.clone()),
                });
                return Err(CommitFailure::Conflict(p.conflicts));
            }
            Err(e) => {
                // once a conflict is known, later errors are usually its consequence
                if !p.conflicts.is_empty() {
                    return Err(CommitFailure::Conflict(p.conflicts));
                }
                return Err(reject(e, Some(i)));
            }
        }
    }
    if !p.conflicts.is_empty() {
        return Err(CommitFailure::Conflict(p.conflicts));
    }
    add_owns_edges(&mut p).map_err(|e| reject(e, None))?;
    if env.import {
        check_deferred_refs(&p).map_err(|e| reject(e, None))?;
    }
    Ok(CommitPlan { overlay: p.ov, ops, lock_targets: p.lock_targets, reports: p.reports, warnings: p.warnings, read_denied: p.read_denied })
}

/// Import defers reference checks; every local blocking reference staged by
/// the batch must end up pointing at a live entity.
fn check_deferred_refs(p: &Planner) -> WsResult<()> {
    for r in &p.ov.ref_add {
        if r.blocks_delete() && r.dst_workspace_id.is_empty() && !r.dst_entity_id.is_empty() {
            if !p.ov.entity(&r.dst_entity_id)?.is_some_and(|e| e.alive()) {
                return Err(WsError::new(Code::ReferenceBroken, format!("{} references missing entity {}", r.src_entity_id, r.dst_entity_id)));
            }
        }
    }
    Ok(())
}

fn dispatch(p: &mut Planner, name: &str, op: &Value) -> OpResult {
    if !op.is_object() {
        return Err(WsError::invalid_op("operation must be an object"));
    }
    match name {
        "entity.create" => entity_create(p, op),
        "entity.delete" => entity_delete(p, op),
        "entity.restore" => entity_restore(p, op),
        "entity.rename" => entity_rename(p, op),
        "entity.set_keys" => entity_set_keys(p, op, false),
        "entity.unset_keys" => entity_set_keys(p, op, true),
        "entity.set_write_policy" => entity_set_write_policy(p, op),
        "entity.set_derived" => entity_set_derived(p, op),
        "tree.move" => tree_move(p, op),
        "tree.place" => tree_place(p, op),
        "tree.clone" => tree_clone(p, op),
        "entity.clone" => entity_clone(p, op),
        "entity.purge" => entity_purge(p, op),
        "data.pin" => data_pin(p, op),
        "data.restore_owns" => data_restore_owns(p, op),
        "dir.set_member" => dir_set_member(p, op),
        "dir.remove_member" => dir_remove_member(p, op),
        "dir.replace" => dir_replace(p, op),
        n if n.starts_with("table.") => crate::plan_table::dispatch(p, n, op),
        n if n.starts_with("richtext.") => crate::plan_richtext::dispatch(p, n, op),
        _ => Err(WsError::invalid_op(format!("unknown operation {name:?}"))),
    }
}

// ---------------------------------------------------------------------------
// tree rules
// ---------------------------------------------------------------------------

fn container_kind(e: &EntityRow) -> &str {
    e.payload.get("kind").and_then(Value::as_str).unwrap_or("")
}

/// Which entities may be children of which (phase two §4.5: two trees).
///
/// * `root` holds only the system nodes (created with the Workspace, never by an operation).
/// * `data` / `folder`: folders and data entities (annotations of any scope); the data tree.
/// * the `shows` system folder: presentation paths and Viewports, nothing else (third phase §6.2);
///   they are never placed anywhere else.
/// * `surfaces`: Surfaces only.
/// * `surface` / `group`: Cells, UI groups and controls only; the BlockTree holds no data.
/// * a control (`mindmap`, 脑图方案 R2/R3/R24): Cells, groups and other controls; inside a control a Cell may hold
///   Cells and containers too (a node's children). `in_control` says the parent is a Cell laid out by a control.
/// * a TableSource holds the rich text bodies of its records.
pub fn child_allowed(parent: &EntityRow, in_control: bool, child_type: &str, child_kind: Option<&str>, child_scope: &str) -> bool {
    if parent.entity_id == SHOWS_ID {
        return is_show_type(child_type) && child_scope == SCOPE_SHARED;
    }
    let block = |child_type: &str| match child_type {
        TYPE_CONTAINER => child_kind.is_some_and(|k| k == "group" || crate::types::is_control_kind(k)),
        TYPE_CELL => child_scope == SCOPE_SHARED,
        _ => false,
    };
    match parent.type_id.as_str() {
        TYPE_CONTAINER => match container_kind(parent) {
            "data" | "folder" => match child_type {
                TYPE_CONTAINER => child_kind == Some("folder"),
                TYPE_ANNOTATION => true,
                t => is_data_type(t) && child_scope == SCOPE_SHARED,
            },
            "surfaces" => child_type == TYPE_CONTAINER && child_kind == Some("surface"),
            "surface" | "group" => block(child_type),
            k if crate::types::is_control_kind(k) => block(child_type),
            _ => false,
        },
        TYPE_CELL if in_control => block(child_type),
        TYPE_TABLE => child_type == TYPE_RICHTEXT,
        _ => false,
    }
}

/// `child_allowed` for a parent in the current state: whether a Cell parent sits inside a control is read from the tree.
fn child_allowed_here(p: &Planner, parent: &EntityRow, child_type: &str, child_kind: Option<&str>, child_scope: &str) -> WsResult<bool> {
    let in_control = parent.type_id == TYPE_CELL && crate::types::in_tree_control(&p.ov, &parent.entity_id)?;
    Ok(child_allowed(parent, in_control, child_type, child_kind, child_scope))
}

/// What a placement is judged against besides its own shape (脑图方案 §5.1–§5.2): where it sits and what it is.
#[derive(Clone, Copy, Default)]
pub struct PlaceRules<'a> {
    /// The connector payload, for a line.
    pub connector: Option<&'a JsonMap>,
    pub internal: bool,
    /// The parent's layout is a control's: `x` / `y` may be `"auto"`.
    pub laid_out: bool,
    /// `w` / `h` may be `"auto"`: a text view inside a control (its title gives the size), or a control itself (its
    /// box wraps its children).
    pub auto_size: bool,
}

impl<'a> PlaceRules<'a> {
    /// The rules for `type_id` / `payload` placed under `parent_id`.
    fn new(p: &Planner, parent_id: &str, type_id: &str, payload: &'a JsonMap) -> WsResult<PlaceRules<'a>> {
        let connector = crate::types::is_connector(type_id, payload).then_some(payload);
        // a `fixed` composite keeps its children where they are put: none of them is laid out
        let host = crate::types::enclosing_container(&p.ov, parent_id)?;
        let fixed = match &host {
            Some(c) if container_kind(c) == crate::types::COMPOSITE => crate::types::composite_algorithm(&p.ov, c)?.as_deref() == Some("fixed"),
            _ => false,
        };
        let laid_out = !fixed && host.as_ref().is_some_and(|c| crate::types::is_control_kind(container_kind(c)));
        let control = type_id == TYPE_CONTAINER && payload.get("kind").and_then(Value::as_str).is_some_and(crate::types::is_control_kind);
        let text = type_id == TYPE_CELL && payload.get("view").and_then(|v| v.get("type")).and_then(Value::as_str).is_some_and(crate::types::is_text_view);
        Ok(PlaceRules { connector, internal: p.env.internal, laid_out, auto_size: control || (laid_out && text) })
    }
}

/// Free-layout placement: `{ x, y, w, h }` relative to the parent container (phase two §8.2).
/// Stacking order is the sibling `order_key`; there is no `z`.
/// `rotation` (optional, degrees clockwise about the centre, `[0, 360)`) is layout like x / y: it is
/// relative to the parent, written with the same capability and merged the same way.
///
/// `x`, `y`, `w`, `h` may each be the string `"auto"` (脑图方案 §5.1, R5/R6/R19): the value is not stored but
/// computed when shown by the control laying the Block out. Only a Block under a control may say so (`laid_out`),
/// and only a text view or a control sizes itself (`auto_size`); writing a number pins that component.
///
/// A connector (`connector` = its payload) stores the bounding box of its two stored endpoint positions
/// (连接线方案 §4.2): a horizontal or vertical line has `w` or `h` 0, there is no rotation, and two coordinate
/// ends must not collapse into one point. That last rule depends on the payload at the time, so it is not
/// applied to internal callers: undo may put back a box written while the ends were still bound.
pub fn check_placement(v: &Value, rules: PlaceRules) -> WsResult<()> {
    let o = v.as_object().ok_or_else(|| WsError::invalid_schema("placement must be an object"))?;
    let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|f| f.is_finite());
    if let Some(payload) = rules.connector {
        let ok = o.keys().all(|k| matches!(k.as_str(), "x" | "y" | "w" | "h"))
            && num("x").is_some()
            && num("y").is_some()
            && num("w").is_some_and(|w| w >= 0.0)
            && num("h").is_some_and(|h| h >= 0.0);
        if !ok {
            return Err(WsError::invalid_schema("a connector's placement must be { x, y, w >= 0, h >= 0 } with finite numbers and no rotation"));
        }
        let coordinate = |k: &str| payload.get(k).is_none_or(Value::is_null);
        if !rules.internal && coordinate("start") && coordinate("end") && num("w").unwrap_or(0.0) + num("h").unwrap_or(0.0) <= 0.0 {
            return Err(WsError::invalid_schema("a connector with two coordinate ends cannot be a single point"));
        }
        return Ok(());
    }
    let auto = |k: &str| o.get(k).and_then(Value::as_str) == Some("auto");
    for k in ["x", "y"] {
        if auto(k) && !rules.laid_out {
            return Err(WsError::invalid_schema(format!("placement.{k} can be \"auto\" only on a Block laid out by a control")));
        }
    }
    for k in ["w", "h"] {
        if auto(k) && !rules.auto_size {
            return Err(WsError::invalid_schema(format!(
                "placement.{k} can be \"auto\" only on a control or on a text view inside one; other Blocks need a numeric size"
            )));
        }
    }
    let ok = o.keys().all(|k| matches!(k.as_str(), "x" | "y" | "w" | "h" | "rotation"))
        && (num("x").is_some() || auto("x"))
        && (num("y").is_some() || auto("y"))
        && (num("w").is_some_and(|w| w > 0.0) || auto("w"))
        && (num("h").is_some_and(|h| h > 0.0) || auto("h"))
        && (!o.contains_key("rotation") || num("rotation").is_some_and(|r| (0.0..360.0).contains(&r)));
    if ok {
        Ok(())
    } else {
        Err(WsError::invalid_schema("placement must be { x, y, w > 0, h > 0, rotation? in [0, 360) } with finite numbers or \"auto\" (stacking order is order_key)"))
    }
}

/// Does the stored placement say `"auto"` anywhere?
pub fn has_auto(placement: Option<&Value>) -> bool {
    placement.and_then(Value::as_object).is_some_and(|o| o.values().any(|v| v.as_str() == Some("auto")))
}

/// Is `id` inside the data tree (under `data`)? Used by annotation rules and the UI's "move to" checks.
pub fn in_data_tree(ctx: &dyn ReadCtx, id: &str) -> WsResult<bool> {
    let mut cur = id.to_string();
    for _ in 0..4096 {
        if cur == DATA_ID {
            return Ok(true);
        }
        match ctx.edge(&cur)? {
            Some(e) => cur = e.parent_id,
            None => return Ok(false),
        }
    }
    Ok(false)
}

/// The Surface `id` is drawn on: the first `surface` container walking up the tree (the entity itself
/// included). `None` outside the BlockTree.
pub fn surface_of(ctx: &dyn ReadCtx, id: &str) -> WsResult<Option<EntityRow>> {
    let mut cur = id.to_string();
    for _ in 0..4096 {
        if let Some(e) = ctx.entity(&cur)? {
            if e.type_id == TYPE_CONTAINER && container_kind(&e) == "surface" {
                return Ok(Some(e));
            }
        }
        match ctx.edge(&cur)? {
            Some(edge) => cur = edge.parent_id,
            None => return Ok(None),
        }
    }
    Ok(None)
}

/// A connector needs its box everywhere it is placed: the line is drawn from it.
fn connector_placement(op: &Value, rules: PlaceRules) -> WsResult<Option<Value>> {
    match op.get("placement") {
        None | Some(Value::Null) if rules.connector.is_some() => Err(WsError::invalid_schema("a connector needs a placement").at("/placement")),
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            check_placement(v, rules)?;
            Ok(Some(v.clone()))
        }
    }
}

fn entity_create(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "type_id", "schema_version", "schema_id", "name", "parent_id", "order_key", "placement", "payload", "scope"])?;
    let id = get_str(op, "entity_id")?;
    check_id("entity_id", id)?;
    let type_id = get_str(op, "type_id")?;
    let schema_version = op.get("schema_version").and_then(Value::as_u64).unwrap_or(1) as u32;
    let schema_id = match op.get("schema_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() && s.len() <= 128 => Some(s.clone()),
        Some(_) => return Err(WsError::invalid_schema("schema_id must be a short string").at("/schema_id")),
    };
    let parent_id = get_str(op, "parent_id")?;
    let order_key = get_str(op, "order_key")?;
    check_order_key(order_key)?;
    let payload = match op.get("payload") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(o)) => o.clone(),
        Some(_) => return Err(WsError::invalid_schema("payload must be an object").at("/payload")),
    };
    if p.ov.entity(id)?.is_some() {
        return Err(WsError::sub(Code::InvalidOperation, "ID_CONFLICT", format!("entity_id {id} already exists")));
    }
    if is_system_id(id) {
        return Err(WsError::invalid_op(format!("{id} is a system entity")));
    }
    let scope = match op.get("scope").and_then(Value::as_str) {
        None | Some("shared") => SCOPE_SHARED.to_string(),
        Some("personal") if type_id == TYPE_ANNOTATION => format!("user:{}", p.env.principal),
        Some(_) => return Err(WsError::invalid_op("scope must be shared, or personal for annotations")),
    };
    let known = supports_schema_version(type_id, schema_version);
    if !known && !p.env.internal {
        let code = if is_known_type(type_id) { Code::UnsupportedVersion } else { Code::MissingExtension };
        return Err(WsError::new(code, format!("cannot create {type_id} v{schema_version}")));
    }
    let parent = p.entity_alive(parent_id)?;
    let kind = payload.get("kind").and_then(Value::as_str);
    if !child_allowed_here(p, &parent, type_id, kind, &scope)? && !(p.env.internal && !known) {
        return Err(WsError::sub(Code::InvalidOperation, "CHILD_NOT_ALLOWED", format!("{type_id} cannot be a child of {parent_id}")));
    }
    if type_id == TYPE_ANNOTATION {
        p.require(parent_id, Cap::Comment)?;
    } else if type_id == TYPE_CELL && !p.can(parent_id, Cap::Structure)? && own_note_payload(&p.ov, &p.env.principal, &payload)? {
        // the caller's own note Block: `comment` puts it on the canvas (see `own_note_block`)
        p.require(parent_id, Cap::Comment)?;
    } else {
        p.require(parent_id, Cap::Structure)?;
        p.need_lock(&parent);
    }
    let name = match op.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) => {
            check_name(n)?;
            if scope == SCOPE_SHARED && p.name_taken(parent_id, n, id)? {
                return Err(WsError::sub(Code::InvalidOperation, "NAME_CONFLICT", format!("name {n:?} already used under {parent_id}")));
            }
            Some(n.clone())
        }
        Some(_) => return Err(WsError::invalid_op("name must be a string")),
    };
    let placement = connector_placement(op, PlaceRules::new(p, parent_id, type_id, &payload)?)?;
    let seq = p.seq();
    let mut row = EntityRow {
        entity_id: id.to_string(),
        type_id: type_id.to_string(),
        schema_version,
        schema_id,
        scope,
        name,
        write_policy: POLICY_OPEN.to_string(),
        payload: Map::new(),
        key_revs: BTreeMap::new(),
        derived: None,
        created_seq: seq,
        meta_rev: seq,
        content_rev: seq,
        life_rev: seq,
        deleted_seq: None,
    };
    // the edge exists before type initialization so adapters can see the tree position
    p.ov.put_edge(TreeEdge { child_id: id.to_string(), parent_id: parent_id.to_string(), order_key: order_key.to_string(), placement, struct_rev: seq });
    let mut norm = op.clone();
    if known {
        let stored_payload = crate::types::init_entity(p, &mut row, payload)?;
        // history and replicas get the payload in its replayable form
        norm["payload"] = stored_payload;
    } else {
        row.payload = payload; // unknown extension: kept verbatim (V20)
    }
    for k in row.payload.keys() {
        row.key_revs.insert(k.clone(), seq);
    }
    let refs = crate::types::entity_refs(&row)?;
    p.ov.set_refs(&BTreeSet::new(), &refs);
    p.ov.put_entity(row);
    p.touch(id, None, "created");
    let inverse = json!({ "op": "entity.delete", "entity_id": id, "subtree": "reject_if_children", "expect": { "rev": seq } });
    Ok((norm, Some(vec![inverse])))
}

/// Alive structural descendants of `id` (depth-first, parents before children).
fn alive_descendants(p: &Planner, id: &str, out: &mut Vec<EntityRow>) -> WsResult<()> {
    for edge in p.ov.children(id)? {
        if let Some(e) = p.ov.entity(&edge.child_id)? {
            if e.alive() {
                out.push(e);
                alive_descendants(p, &edge.child_id, out)?;
            }
        }
    }
    Ok(())
}

fn entity_delete(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "subtree", "expect"])?;
    let id = get_str(op, "entity_id")?;
    // deleted earlier in this very Commit (a cascade, or the undo of a cascade): the intent is met
    if p.entity_opt(id)?.is_some_and(|e| e.deleted_seq == Some(p.seq())) {
        return Ok((op.clone(), Some(vec![])));
    }
    let e = p.entity_alive(id)?;
    if is_system_id(id) {
        return Err(WsError::invalid_op(format!("{id} is a system entity and cannot be deleted")));
    }
    if e.type_id == TYPE_ANNOTATION {
        // own annotations need `comment`; other people's shared ones need `manage`
        let mine = crate::types::annotation_author(&e) == Some(p.env.principal.as_str());
        p.require(id, if mine { Cap::Comment } else { Cap::Manage })?;
    } else if !p.by_comment(&e, id, Cap::Delete, false)? {
        p.require(id, Cap::Delete)?;
        p.need_parent_lock(id)?;
    }
    if !p.check_rev(op.get("expect"), e.life_rev, None, id, None, || None)? {
        return Ok((op.clone(), None));
    }
    let mut descendants = Vec::new();
    alive_descendants(p, id, &mut descendants)?;
    let listed: BTreeSet<String> = match op.get("subtree") {
        None => BTreeSet::new(),
        Some(Value::String(s)) if s == "reject_if_children" => BTreeSet::new(),
        Some(Value::Object(o)) if o.len() == 1 && o.get("delete").is_some_and(Value::is_array) => {
            o["delete"].as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect()
        }
        Some(_) => return Err(WsError::invalid_op("subtree must be \"reject_if_children\" or { \"delete\": [...] }")),
    };
    // personal-scope entities of other principals are invisible to the caller and go with their parent
    let unlisted: Vec<&EntityRow> =
        descendants.iter().filter(|d| !listed.contains(&d.entity_id) && p.access.sees_scope(d)).collect();
    if !unlisted.is_empty() {
        let ids: Vec<&str> = unlisted.iter().map(|d| d.entity_id.as_str()).collect();
        if listed.is_empty() && op.get("subtree").map_or(true, Value::is_string) {
            return Err(WsError::invalid_op("entity has children; list the subtree to delete it").with_data(json!({ "children": ids })));
        }
        // someone moved or created something under this container after the caller looked
        p.conflict(Code::RevisionConflict, None, id, None, json!(listed.len()), json!(descendants.len()), None);
        if let Some(c) = p.conflicts.last_mut() {
            c.detail = Some(format!("subtree contains entities not listed for deletion: {}", ids.join(", ")));
        }
        return Ok((op.clone(), None));
    }
    let mut doomed = vec![e];
    doomed.extend(descendants);
    let doomed_ids: BTreeSet<String> = doomed.iter().map(|d| d.entity_id.clone()).collect();
    // the pre-check names only referrers the caller may read; the rest is reported as a fact without
    // ids, names or counts (phase two §4.3)
    let (mut referrers, mut hidden) = (Vec::new(), false);
    for d in &doomed {
        for r in p.ov.refs_to(&d.entity_id)? {
            if r.blocks_delete() && !doomed_ids.contains(&r.src_entity_id) {
                if let Some(src) = p.ov.entity(&r.src_entity_id)?.filter(|s| s.alive()) {
                    if p.access.can_read(&p.ov, &src)? {
                        referrers.push(json!({ "entity_id": r.src_entity_id, "selector": r.src_selector, "kind": r.kind, "target": d.entity_id }));
                    } else {
                        hidden = true;
                    }
                }
            }
        }
    }
    if !referrers.is_empty() || hidden {
        return Err(WsError::new(Code::ReferenceBroken, "entity is still referenced").with_data(json!({ "referrers": referrers, "hidden_referrers": hidden })));
    }
    let seq = p.seq();
    let mut inverse = Vec::new();
    for mut d in doomed {
        if d.entity_id != id {
            p.require(&d.entity_id, Cap::Delete)?;
        }
        d.deleted_seq = Some(seq);
        d.life_rev = seq;
        inverse.push(json!({ "op": "entity.restore", "entity_id": d.entity_id, "expect": { "rev": seq } }));
        p.touch(&d.entity_id, None, "deleted");
        p.ov.put_entity(d);
    }
    // own data nobody else shows goes with its Block (D05); the expansion is written into the
    // normalized operation so the log never has to decide it again (D20)
    let mut norm = op.clone();
    let orphans = orphaned_own_data(p, &doomed_ids)?;
    if !orphans.is_empty() {
        let mut cascade = Vec::new();
        for data_id in orphans {
            let Some(data) = p.ov.entity(&data_id)?.filter(|d| d.alive()) else { continue };
            let mut kids = Vec::new();
            alive_descendants(p, &data_id, &mut kids)?;
            let mut sub = json!({ "op": "entity.delete", "entity_id": data_id, "expect": { "rev": data.life_rev } });
            if !kids.is_empty() {
                sub["subtree"] = json!({ "delete": kids.iter().map(|k| k.entity_id.clone()).collect::<Vec<_>>() });
            }
            let (done, inv) = entity_delete(p, &sub)?;
            if let Some(inv) = inv {
                inverse.extend(inv);
                cascade.push(done);
            }
        }
        if !cascade.is_empty() {
            norm["cascade"] = Value::Array(cascade);
        }
    }
    Ok((norm, Some(inverse)))
}

fn entity_restore(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let mut e = p
        .entity_opt(id)?
        .ok_or_else(|| WsError::not_found(format!("entity {id} not found")))?;
    if e.alive() {
        return Err(WsError::invalid_op(format!("entity {id} is not deleted")));
    }
    if e.payload.contains_key(PURGED_KEY) {
        return Err(WsError::new(Code::ResourceReclaimed, format!("entity {id} was purged: its content is gone for good")));
    }
    // the author brings back its own annotation or note Block (undoing their deletion) with `comment`
    let by_comment = p.by_comment(&e, id, Cap::Delete, true)?;
    if !by_comment {
        p.require(id, Cap::Delete)?;
    }
    if !p.check_rev(op.get("expect"), e.life_rev, None, id, None, || None)? {
        return Ok((op.clone(), None));
    }
    let edge = p.ov.edge(id)?.ok_or_else(|| WsError::new(Code::ReferenceBroken, "entity has no parent"))?;
    let parent = p.ov.entity(&edge.parent_id)?.filter(|x| x.alive()).ok_or_else(|| {
        WsError::new(Code::ReferenceBroken, format!("parent {} is deleted", edge.parent_id))
            .with_data(json!({ "entity_id": edge.parent_id }))
    })?;
    if !by_comment {
        p.need_lock(&parent);
    }
    if let Some(n) = &e.name {
        if e.scope == SCOPE_SHARED && p.name_taken(&edge.parent_id, n, id)? {
            return Err(WsError::sub(Code::InvalidOperation, "NAME_CONFLICT", format!("name {n:?} is taken again")));
        }
    }
    let seq = p.seq();
    e.deleted_seq = None;
    e.life_rev = seq;
    p.ov.put_entity(e);
    p.touch(id, None, "restored");
    let inverse = json!({ "op": "entity.delete", "entity_id": id, "subtree": "reject_if_children", "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inverse])))
}

fn entity_rename(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "name", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let mut e = p.entity_alive(id)?;
    if is_system_id(id) {
        return Err(WsError::invalid_op(format!("{id} is a system entity and cannot be renamed")));
    }
    p.require(id, Cap::Structure)?;
    p.need_parent_lock(id)?;
    let old = e.name.clone();
    if !p.check_rev(op.get("expect"), e.meta_rev, None, id, None, || Some(json!(old)))? {
        return Ok((op.clone(), None));
    }
    let name = match op.get("name") {
        Some(Value::Null) => None,
        Some(Value::String(n)) => {
            check_name(n)?;
            if let Some(edge) = p.ov.edge(id)? {
                if e.scope == SCOPE_SHARED && p.name_taken(&edge.parent_id, n, id)? {
                    return Err(WsError::sub(Code::InvalidOperation, "NAME_CONFLICT", format!("name {n:?} already used")));
                }
            }
            Some(n.clone())
        }
        _ => return Err(WsError::invalid_op("name must be a string or null").at("/name")),
    };
    let seq = p.seq();
    e.name = name;
    e.meta_rev = seq;
    p.ov.put_entity(e);
    p.touch(id, None, "renamed");
    let inverse = json!({ "op": "entity.rename", "entity_id": id, "name": old, "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inverse])))
}

fn entity_set_write_policy(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "policy", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let mut e = p.entity_alive(id)?;
    p.require(id, Cap::Manage)?;
    let policy = get_str(op, "policy")?;
    if policy != POLICY_OPEN && policy != POLICY_LOCK {
        return Err(WsError::invalid_op("policy must be open or lock_required"));
    }
    let old = e.write_policy.clone();
    if !p.check_rev(op.get("expect"), e.meta_rev, None, id, None, || Some(json!(old)))? {
        return Ok((op.clone(), None));
    }
    let seq = p.seq();
    e.write_policy = policy.to_string();
    e.meta_rev = seq;
    p.ov.put_entity(e);
    p.touch(id, None, "renamed");
    let inverse = json!({ "op": "entity.set_write_policy", "entity_id": id, "policy": old, "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inverse])))
}

fn entity_set_keys(p: &mut Planner, op: &Value, unset: bool) -> OpResult {
    only_keys(op, &["entity_id", "keys"])?;
    let id = get_str(op, "entity_id")?;
    let mut e = p.entity_alive(id)?;
    if let Some(d) = e.degraded() {
        let code = if d == "MISSING_EXTENSION" { Code::MissingExtension } else { Code::UnsupportedVersion };
        return Err(WsError::new(code, format!("content of {id} cannot be edited ({d})")));
    }
    if e.type_id == TYPE_RICHTEXT && get_array(op, "keys")?.iter().any(|k| k.get("key").and_then(Value::as_str) != Some("source")) {
        return Err(WsError::invalid_op("rich text content is edited with richtext.* operations"));
    }
    // a sourced entity keeps its content for its refresh; its Source and presentation keys stay editable
    if crate::types::has_source(&e) && get_array(op, "keys")?.iter().any(|k| k.get("key").and_then(Value::as_str).is_some_and(|k| !crate::types::SOURCE_FREE_KEYS.contains(&k))) {
        crate::types::require_source_writable(p, &e)?;
    }
    let mut by_comment = false;
    if e.type_id == TYPE_ANNOTATION {
        let mine = crate::types::annotation_author(&e) == Some(p.env.principal.as_str());
        p.require(id, if mine { Cap::Comment } else { Cap::Manage })?;
    } else if p.by_comment(&e, id, Cap::Update, false)? {
        by_comment = true;
    } else {
        p.require(id, Cap::Update)?;
        p.need_lock(&e);
    }
    let items = get_array(op, "keys")?;
    if items.is_empty() {
        return Err(WsError::invalid_op("keys must not be empty"));
    }
    let seq = p.seq();
    let old_refs = crate::types::entity_refs(&e)?;
    let before = e.clone();
    let (mut restore_set, mut restore_unset, mut changed) = (Vec::new(), Vec::new(), Vec::new());
    let mut ok = true;
    for (i, item) in items.iter().enumerate() {
        let key = get_str(item, "key")?;
        if key.is_empty() || key.starts_with('#') || key.len() > 128 {
            return Err(WsError::invalid_op(format!("invalid key {key:?}")));
        }
        let old = e.payload.get(key).cloned();
        let selector = json!({ "kind": "doc_key", "key": key });
        if !p.check_rev(item.get("expect"), e.key_rev(key), Some(i), id, sel(selector.clone()), || old.clone())? {
            ok = false;
            continue;
        }
        if unset {
            e.payload.remove(key);
        } else {
            let value = item.get("value").ok_or_else(|| WsError::invalid_op("value required").at(format!("/keys/{i}/value")))?;
            e.payload.insert(key.to_string(), value.clone());
        }
        e.key_revs.insert(key.to_string(), seq);
        match old {
            Some(v) => restore_set.push(json!({ "key": key, "value": v, "expect": { "rev": seq } })),
            None => restore_unset.push(json!({ "key": key, "expect": { "rev": seq } })),
        }
        changed.push(key.to_string());
    }
    if !ok {
        return Ok((op.clone(), None));
    }
    crate::types::validate_update(p, &before, &mut e, &changed)?;
    if by_comment && !own_note_block(&p.ov, &p.env.principal, &e)? {
        return Err(WsError::denied("update capability required: the note Block would no longer show your own note"));
    }
    // keys the type derived or dropped as a consequence are versioned like explicit writes
    let derived: Vec<String> = before.payload.keys().chain(e.payload.keys()).filter(|k| !changed.contains(*k) && before.payload.get(*k) != e.payload.get(*k)).cloned().collect();
    for k in derived {
        e.key_revs.insert(k, seq);
    }
    let new_refs = crate::types::entity_refs(&e)?;
    p.ov.set_refs(&old_refs, &new_refs);
    e.content_rev = seq;
    let class = crate::types::change_class(&e.type_id);
    for k in &changed {
        p.touch(id, sel(json!({ "kind": "doc_key", "key": k })), class);
    }
    // the stored payload may have been normalized: history records what was stored
    let mut norm = op.clone();
    if !unset {
        for item in norm["keys"].as_array_mut().unwrap() {
            if let Some(v) = item["key"].as_str().and_then(|k| e.payload.get(k)) {
                item["value"] = v.clone();
            }
        }
    }
    p.ov.put_entity(e);
    let mut inverse = Vec::new();
    if !restore_set.is_empty() {
        inverse.push(json!({ "op": "entity.set_keys", "entity_id": id, "keys": restore_set }));
    }
    if !restore_unset.is_empty() {
        inverse.push(json!({ "op": "entity.unset_keys", "entity_id": id, "keys": restore_unset }));
    }
    Ok((norm, Some(inverse)))
}

/// `entity.set_derived`: record (or clear) the generation dependency of a result entity
/// (phase two §7.3). Auto-merge: the wish application guards consistency with its read-set
/// preconditions, and a later record simply replaces the earlier one. `generated_rev` is the
/// entity's content version at this point (the content written in the same commit has `seq`).
fn entity_set_derived(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "derived", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let mut e = p.entity_alive(id)?;
    if e.type_id == TYPE_CONTAINER || e.type_id == TYPE_CELL {
        return Err(WsError::invalid_op("only data entities carry a dependency record"));
    }
    p.require(id, Cap::Update)?;
    p.need_lock(&e);
    if !p.check_rev_opt(op.get("expect"), e.meta_rev, None, id, None, || None)? {
        return Ok((op.clone(), None));
    }
    let derived = match op.get("derived") {
        None | Some(Value::Null) => None,
        Some(v) => Some(crate::types::check_derived(p, v, e.content_rev)?),
    };
    let seq = p.seq();
    let old_refs = crate::types::entity_refs(&e)?;
    let inverse = json!({ "op": "entity.set_derived", "entity_id": id, "derived": e.derived.clone().unwrap_or(Value::Null), "expect": { "rev": seq } });
    e.derived = derived;
    e.meta_rev = seq;
    let new_refs = crate::types::entity_refs(&e)?;
    p.ov.set_refs(&old_refs, &new_refs);
    let mut norm = op.clone();
    norm["derived"] = e.derived.clone().unwrap_or(Value::Null);
    p.ov.put_entity(e);
    p.touch(id, None, "derived");
    Ok((norm, Some(vec![inverse])))
}

/// `tree.move { entity_id, new_parent_id, order_key, placement?, slot?, expect? }`. `slot` (a name, or `null` to drop
/// it) rewrites the Block's role together with its place: into a composite a Block needs one, out of it none.
fn tree_move(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "new_parent_id", "order_key", "placement", "slot", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let new_parent_id = get_str(op, "new_parent_id")?;
    let order_key = get_str(op, "order_key")?;
    check_order_key(order_key)?;
    let e = p.entity_alive(id)?;
    if is_system_id(id) {
        return Err(WsError::invalid_op(format!("{id} is a system entity and cannot be moved")));
    }
    let new_parent = p.entity_alive(new_parent_id)?;
    let kind = e.payload.get("kind").and_then(Value::as_str);
    if !child_allowed_here(p, &new_parent, &e.type_id, kind, &e.scope)? {
        return Err(WsError::sub(Code::InvalidOperation, "CHILD_NOT_ALLOWED", format!("{} cannot be a child of {new_parent_id}", e.type_id)));
    }
    // a node keeps its children only where a tree control lays Blocks out (R3): elsewhere a Cell is a leaf again
    if e.type_id == TYPE_CELL && !crate::types::in_tree_control(&p.ov, new_parent_id)? {
        let mut kids = Vec::new();
        alive_descendants(p, id, &mut kids)?;
        if !kids.is_empty() {
            return Err(WsError::sub(Code::InvalidOperation, "CHILD_NOT_ALLOWED", format!("{id} holds Blocks: only a Block inside a control can")));
        }
    }
    let mut edge = p.ov.edge(id)?.ok_or_else(|| WsError::invalid_op("entity has no tree edge"))?;
    let from_by_comment = p.by_comment(&e, id, Cap::Structure, false)?;
    if !from_by_comment {
        p.require(id, Cap::Structure)?;
    }
    let to_by_comment = p.by_comment(&e, new_parent_id, Cap::Structure, false)?;
    if !to_by_comment {
        p.require(new_parent_id, Cap::Structure)?;
    }
    // cycle check: the new parent must not be the entity or inside its subtree
    let mut cur = new_parent_id.to_string();
    loop {
        if cur == id {
            return Err(WsError::sub(Code::InvalidOperation, "TREE_CYCLE", "move would create a cycle"));
        }
        match p.ov.edge(&cur)? {
            Some(up) => cur = up.parent_id,
            None => break,
        }
    }
    if !p.check_rev_opt(op.get("expect"), edge.struct_rev, None, id, None, || None)? {
        return Ok((op.clone(), None));
    }
    if let Some(n) = &e.name {
        if new_parent_id != edge.parent_id && e.scope == SCOPE_SHARED && p.name_taken(new_parent_id, n, id)? {
            return Err(WsError::sub(Code::InvalidOperation, "NAME_CONFLICT", format!("name {n:?} already used under {new_parent_id}")));
        }
    }
    if !from_by_comment {
        p.need_parent_lock(id)?;
    }
    if !to_by_comment {
        p.need_lock(&new_parent);
    }
    // a connector moved into a flow page keeps its data (not rendered there, §8.3): only the box is judged; a Block
    // moved out of a control cannot keep an "auto" placement (R9: the Desktop never moves one out)
    let placement = connector_placement(op, PlaceRules::new(p, new_parent_id, &e.type_id, &e.payload)?)?;
    let seq = p.seq();
    let mut inverse = json!({ "op": "tree.move", "entity_id": id, "new_parent_id": edge.parent_id, "order_key": edge.order_key,
                              "placement": edge.placement, "expect": { "rev": seq } });
    edge.parent_id = new_parent_id.to_string();
    edge.order_key = order_key.to_string();
    edge.placement = placement;
    edge.struct_rev = seq;
    p.ov.put_edge(edge);
    let mut e = e;
    if let Some(slot) = op.get("slot") {
        if !slot.is_null() && !slot.as_str().is_some_and(crate::types::is_slot) {
            return Err(WsError::invalid_schema("slot must match ^[a-z0-9][a-z0-9_-]{0,31}$ or be null").at("/slot"));
        }
        if e.type_id != TYPE_CELL && !crate::types::is_block_container(&e) {
            return Err(WsError::invalid_op("only a Block or a Block container carries a slot"));
        }
        let old = e.payload.get("slot").cloned().unwrap_or(Value::Null);
        if &old != slot {
            inverse["slot"] = old;
            match slot {
                Value::Null => e.payload.remove("slot"),
                s => e.payload.insert("slot".into(), s.clone()),
            };
            e.key_revs.insert("slot".into(), seq);
            p.bump_content(&mut e);
            p.ov.put_entity(e.clone());
            p.touch(id, sel(json!({ "kind": "doc_key", "key": "slot" })), crate::types::change_class(&e.type_id));
        }
    }
    crate::types::check_position(p, &e)?;
    p.touch(id, None, "moved");
    Ok((op.clone(), Some(vec![inverse])))
}

fn tree_place(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "order_key", "placement", "expect"])?;
    let id = get_str(op, "entity_id")?;
    let e = p.entity_alive(id)?;
    if is_system_id(id) {
        return Err(WsError::invalid_op(format!("{id} is a system entity and cannot be placed")));
    }
    let mut edge = p.ov.edge(id)?.ok_or_else(|| WsError::invalid_op("the root cannot be placed"))?;
    let rules = PlaceRules::new(p, &edge.parent_id, &e.type_id, &e.payload)?;
    if !p.by_comment(&e, &edge.parent_id, Cap::Structure, false)? {
        p.require(&edge.parent_id, Cap::Structure)?;
        p.need_parent_lock(id)?;
    }
    if !p.check_rev_opt(op.get("expect"), edge.struct_rev, None, id, None, || None)? {
        return Ok((op.clone(), None));
    }
    let seq = p.seq();
    let mut inverse = json!({ "op": "tree.place", "entity_id": id, "expect": { "rev": seq } });
    let mut change = "placed";
    if let Some(k) = op.get("order_key").filter(|v| !v.is_null()) {
        let k = k.as_str().ok_or_else(|| WsError::invalid_op("order_key must be a string"))?;
        check_order_key(k)?;
        inverse["order_key"] = json!(edge.order_key);
        edge.order_key = k.to_string();
        change = "moved";
    }
    if op.get("placement").is_some() {
        inverse["placement"] = edge.placement.clone().unwrap_or(Value::Null);
        edge.placement = connector_placement(op, rules)?;
    }
    if op.get("order_key").map_or(true, Value::is_null) && op.get("placement").is_none() {
        return Err(WsError::invalid_op("tree.place needs order_key or placement"));
    }
    edge.struct_rev = seq;
    p.ov.put_edge(edge);
    p.touch(id, None, change);
    Ok((op.clone(), Some(vec![inverse])))
}
