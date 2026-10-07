//! Commit planning (design doc §2.5–§2.7): turn one Commit request into a
//! candidate state layered over the current one, without touching storage.
//!
//! Planning runs inside the single writer, so `expect` checks are evaluated
//! here against the current state. All detectable conflicts of a batch are
//! collected; any hard error rejects the whole batch.

use crate::access::{Access, Cap};
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
        if req.protocol_version != PROTOCOL_VERSION {
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
        _ => return Err(WsError::new(Code::MissingExtension, format!("unknown selector kind {kind}"))),
    })
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
];

/// Operations only the undo path / import may submit.
fn is_internal_op(name: &str) -> bool {
    matches!(name, "table.restore_records" | "table.restore_field" | "table.restore_migration")
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
/// * `surfaces`: Surfaces only.
/// * `surface` / `group`: Cells and UI groups only; the BlockTree holds no data.
/// * a TableSource holds the rich text bodies of its records.
pub fn child_allowed(parent: &EntityRow, child_type: &str, child_kind: Option<&str>, child_scope: &str) -> bool {
    match parent.type_id.as_str() {
        TYPE_CONTAINER => match container_kind(parent) {
            "data" | "folder" => match child_type {
                TYPE_CONTAINER => child_kind == Some("folder"),
                TYPE_ANNOTATION => true,
                t => is_data_type(t) && child_scope == SCOPE_SHARED,
            },
            "surfaces" => child_type == TYPE_CONTAINER && child_kind == Some("surface"),
            "surface" | "group" => match child_type {
                TYPE_CONTAINER => child_kind == Some("group"),
                TYPE_CELL => child_scope == SCOPE_SHARED,
                _ => false,
            },
            _ => false,
        },
        TYPE_TABLE => child_type == TYPE_RICHTEXT,
        _ => false,
    }
}

/// Free-layout placement: `{ x, y, w, h }` relative to the parent container (phase two §8.2).
/// Stacking order is the sibling `order_key`; there is no `z`.
/// `rotation` (optional, degrees clockwise about the centre, `[0, 360)`) is layout like x / y: it is
/// relative to the parent, written with the same capability and merged the same way.
///
/// A connector (`connector` = its payload) stores the bounding box of its two stored endpoint positions
/// (连接线方案 §4.2): a horizontal or vertical line has `w` or `h` 0, there is no rotation, and two coordinate
/// ends must not collapse into one point. That last rule depends on the payload at the time, so it is not
/// applied to internal callers: undo may put back a box written while the ends were still bound.
fn check_placement(v: &Value, connector: Option<&JsonMap>, internal: bool) -> WsResult<()> {
    let o = v.as_object().ok_or_else(|| WsError::invalid_schema("placement must be an object"))?;
    let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|f| f.is_finite());
    if let Some(payload) = connector {
        let ok = o.keys().all(|k| matches!(k.as_str(), "x" | "y" | "w" | "h"))
            && num("x").is_some()
            && num("y").is_some()
            && num("w").is_some_and(|w| w >= 0.0)
            && num("h").is_some_and(|h| h >= 0.0);
        if !ok {
            return Err(WsError::invalid_schema("a connector's placement must be { x, y, w >= 0, h >= 0 } with finite numbers and no rotation"));
        }
        let coordinate = |k: &str| payload.get(k).is_none_or(Value::is_null);
        if !internal && coordinate("start") && coordinate("end") && num("w").unwrap_or(0.0) + num("h").unwrap_or(0.0) <= 0.0 {
            return Err(WsError::invalid_schema("a connector with two coordinate ends cannot be a single point"));
        }
        return Ok(());
    }
    let ok = o.keys().all(|k| matches!(k.as_str(), "x" | "y" | "w" | "h" | "rotation"))
        && num("x").is_some()
        && num("y").is_some()
        && num("w").is_some_and(|w| w > 0.0)
        && num("h").is_some_and(|h| h > 0.0)
        && (!o.contains_key("rotation") || num("rotation").is_some_and(|r| (0.0..360.0).contains(&r)));
    if ok {
        Ok(())
    } else {
        Err(WsError::invalid_schema("placement must be { x, y, w > 0, h > 0, rotation? in [0, 360) } with finite numbers (stacking order is order_key)"))
    }
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
fn connector_placement(op: &Value, connector: Option<&JsonMap>, internal: bool) -> WsResult<Option<Value>> {
    match op.get("placement") {
        None | Some(Value::Null) if connector.is_some() => Err(WsError::invalid_schema("a connector needs a placement").at("/placement")),
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            check_placement(v, connector, internal)?;
            Ok(Some(v.clone()))
        }
    }
}

fn entity_create(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "type_id", "schema_version", "name", "parent_id", "order_key", "placement", "payload", "scope"])?;
    let id = get_str(op, "entity_id")?;
    check_id("entity_id", id)?;
    let type_id = get_str(op, "type_id")?;
    let schema_version = op.get("schema_version").and_then(Value::as_u64).unwrap_or(1) as u32;
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
    let known = is_known_type(type_id) && schema_version == 1;
    if !known && !p.env.internal {
        let code = if is_known_type(type_id) { Code::UnsupportedVersion } else { Code::MissingExtension };
        return Err(WsError::new(code, format!("cannot create {type_id} v{schema_version}")));
    }
    let parent = p.entity_alive(parent_id)?;
    let kind = payload.get("kind").and_then(Value::as_str);
    if !child_allowed(&parent, type_id, kind, &scope) && !(p.env.internal && !known) {
        return Err(WsError::sub(Code::InvalidOperation, "CHILD_NOT_ALLOWED", format!("{type_id} cannot be a child of {parent_id}")));
    }
    if type_id == TYPE_ANNOTATION {
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
    let connector = crate::types::is_connector(type_id, &payload).then_some(&payload);
    let placement = connector_placement(op, connector, p.env.internal)?;
    let seq = p.seq();
    let mut row = EntityRow {
        entity_id: id.to_string(),
        type_id: type_id.to_string(),
        schema_version,
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
    let e = p.entity_alive(id)?;
    if is_system_id(id) {
        return Err(WsError::invalid_op(format!("{id} is a system entity and cannot be deleted")));
    }
    if e.type_id == TYPE_ANNOTATION {
        // own annotations need `comment`; other people's shared ones need `manage`
        let mine = crate::types::annotation_author(&e) == Some(p.env.principal.as_str());
        p.require(id, if mine { Cap::Comment } else { Cap::Manage })?;
    } else {
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
    Ok((op.clone(), Some(inverse)))
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
    p.require(id, Cap::Delete)?;
    if !p.check_rev(op.get("expect"), e.life_rev, None, id, None, || None)? {
        return Ok((op.clone(), None));
    }
    let edge = p.ov.edge(id)?.ok_or_else(|| WsError::new(Code::ReferenceBroken, "entity has no parent"))?;
    let parent = p.ov.entity(&edge.parent_id)?.filter(|x| x.alive()).ok_or_else(|| {
        WsError::new(Code::ReferenceBroken, format!("parent {} is deleted", edge.parent_id))
            .with_data(json!({ "entity_id": edge.parent_id }))
    })?;
    p.need_lock(&parent);
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
    if e.type_id == TYPE_RICHTEXT {
        return Err(WsError::invalid_op("rich text content is edited with richtext.* operations"));
    }
    if e.type_id == TYPE_ANNOTATION {
        let mine = crate::types::annotation_author(&e) == Some(p.env.principal.as_str());
        p.require(id, if mine { Cap::Comment } else { Cap::Manage })?;
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

fn tree_move(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "new_parent_id", "order_key", "placement", "expect"])?;
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
    if !child_allowed(&new_parent, &e.type_id, kind, &e.scope) {
        return Err(WsError::sub(Code::InvalidOperation, "CHILD_NOT_ALLOWED", format!("{} cannot be a child of {new_parent_id}", e.type_id)));
    }
    let mut edge = p.ov.edge(id)?.ok_or_else(|| WsError::invalid_op("entity has no tree edge"))?;
    p.require(id, Cap::Structure)?;
    p.require(new_parent_id, Cap::Structure)?;
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
    p.need_parent_lock(id)?;
    p.need_lock(&new_parent);
    // a connector moved into a flow page keeps its data (not rendered there, §8.3): only the box is judged
    let placement = connector_placement(op, crate::types::is_connector(&e.type_id, &e.payload).then_some(&e.payload), p.env.internal)?;
    let seq = p.seq();
    let inverse = json!({ "op": "tree.move", "entity_id": id, "new_parent_id": edge.parent_id, "order_key": edge.order_key,
                          "placement": edge.placement, "expect": { "rev": seq } });
    edge.parent_id = new_parent_id.to_string();
    edge.order_key = order_key.to_string();
    edge.placement = placement;
    edge.struct_rev = seq;
    p.ov.put_edge(edge);
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
    let connector = crate::types::is_connector(&e.type_id, &e.payload).then_some(&e.payload);
    let mut edge = p.ov.edge(id)?.ok_or_else(|| WsError::invalid_op("the root cannot be placed"))?;
    p.require(&edge.parent_id, Cap::Structure)?;
    p.need_parent_lock(id)?;
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
        edge.placement = connector_placement(op, connector, p.env.internal)?;
    }
    if op.get("order_key").map_or(true, Value::is_null) && op.get("placement").is_none() {
        return Err(WsError::invalid_op("tree.place needs order_key or placement"));
    }
    edge.struct_rev = seq;
    p.ov.put_edge(edge);
    p.touch(id, None, change);
    Ok((op.clone(), Some(vec![inverse])))
}
