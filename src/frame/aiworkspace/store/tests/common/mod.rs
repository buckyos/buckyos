#![allow(dead_code)]
//! Shared helpers: a service on a temp folder with a controllable clock, and
//! the `project-workspace` fixture replayed through the service interface.

use aiworkspace_store::objects::ObjectStore;
use aiworkspace_store::workspace::{Caller, CommitOpts, Workspace};
use aiworkspace_store::Service;
use base64::Engine;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

pub const FIXTURE: &str = include_str!("../../../fixtures/project-workspace/commits.json");
pub const T0: i64 = 1_791_100_800_000; // 2026-10-04T08:00:00Z

pub struct Env {
    pub dir: tempfile::TempDir,
    pub svc: Service,
    pub clock: Arc<AtomicI64>,
    pub objects: Arc<ObjectStore>,
}

/// A named store and a bucket index of the test's own, the service's bucket declarations.
pub fn objects(dir: &std::path::Path) -> Arc<ObjectStore> {
    ObjectStore::embedded(&dir.join("store"), buckyos_api::aiworkspace_bucket_decls(), Some(buckyos_api::AIWORKSPACE_STORAGE_QUOTA_MIB)).unwrap()
}

pub fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicI64::new(T0));
    let c = clock.clone();
    let objects = objects(dir.path());
    let svc = Service::open_with_clock(dir.path(), objects.clone(), Arc::new(move || c.load(Ordering::SeqCst))).unwrap();
    Env { dir, svc, clock, objects }
}

impl Env {
    pub fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }
    pub fn reopen(self) -> Env {
        let Env { dir, svc, clock, objects } = self;
        drop(svc);
        let c = clock.clone();
        let svc = Service::open_with_clock(dir.path(), objects.clone(), Arc::new(move || c.load(Ordering::SeqCst))).unwrap();
        Env { dir, svc, clock, objects }
    }
    /// The object a binding of the Workspace owner `owner` holds under `key` of bucket `name`.
    pub fn bound(&self, owner: &str, name: &str, key: &str) -> Option<String> {
        let bucket = aiworkspace_store::objects::bucket_id(owner, name).unwrap();
        self.objects.binding(&bucket, key).unwrap().map(|(obj, _)| obj)
    }
}

/// The old grant shape on the permission model (权限需求 §4): a principal gets an entry of its own; `public` turns
/// anonymous access on and allows `@guest`; `*`, `group:<id>` and `contact:…` become members of a logical group
/// that holds the entry.
pub trait Grants {
    fn grant(&mut self, by: &Caller, subject: &str, scope: Option<&str>, caps: &[String]) -> aiworkspace_core::WsResult<Value>;
    fn revoke(&mut self, by: &Caller, subject: &str, scope: Option<&str>) -> aiworkspace_core::WsResult<Value>;
}

fn grant_subject(ws: &mut Workspace, by: &Caller, subject: &str) -> aiworkspace_core::WsResult<String> {
    if subject == "public" {
        ws.acl_set_anonymous(by, true)?;
        return Ok("@guest".into());
    }
    if subject == "*" || subject.starts_with("group:") || subject.starts_with("contact:") {
        let id: String = format!("g-{}", subject.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect::<String>());
        let id: String = id.chars().take(64).collect();
        ws.acl_set_group(by, &id, None, None)?;
        ws.acl_members(by, &id, &[subject.to_string()], &[])?;
        return Ok(format!("@{id}"));
    }
    Ok(subject.to_string())
}

impl Grants for Workspace {
    fn grant(&mut self, by: &Caller, subject: &str, scope: Option<&str>, caps: &[String]) -> aiworkspace_core::WsResult<Value> {
        let subject = grant_subject(self, by, subject)?;
        self.acl_set(by, &json!([{ "subject": subject, "resource": scope.unwrap_or("/"), "allow": caps }]))
    }
    fn revoke(&mut self, by: &Caller, subject: &str, scope: Option<&str>) -> aiworkspace_core::WsResult<Value> {
        let subject = grant_subject(self, by, subject)?;
        self.acl_set(by, &json!([{ "subject": subject, "resource": scope.unwrap_or("/"), "allow": [], "deny": [] }]))
    }
}

pub fn caps(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

pub fn alice() -> Caller {
    Caller::user("alice")
}
pub fn bob() -> Caller {
    Caller::user("bob")
}

static KEY: AtomicU64 = AtomicU64::new(0);

pub fn request(ws: &Workspace, ops: Value) -> Value {
    json!({ "protocol_version": "1.0", "workspace_id": ws.workspace_id, "epoch": ws.epoch,
            "idempotency_key": format!("t/{}", KEY.fetch_add(1, Ordering::SeqCst)), "session_id": "s1", "operations": ops })
}

pub fn commit(ws: &mut Workspace, who: &Caller, ops: Value) -> Value {
    let req = request(ws, ops);
    ws.commit(&req, who, &CommitOpts::default())
}

pub fn ok(ws: &mut Workspace, who: &Caller, ops: Value) -> Value {
    let r = commit(ws, who, ops);
    assert_eq!(r["status"], "accepted", "{r}");
    r
}

pub fn code(v: &Value) -> &str {
    v["code"].as_str().or_else(|| v["error"]["code"].as_str()).unwrap_or("")
}

/// Create a Workspace owned by alice and replay the fixture into it.
pub fn project(env: &Env) -> String {
    let id = env.svc.create_workspace(&alice(), "项目工作区", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    replay_fixture(&mut ws, &alice());
    id
}

pub fn replay_fixture(ws: &mut Workspace, who: &Caller) {
    let fx: Value = serde_json::from_str(FIXTURE).unwrap();
    let mut text = serde_json::to_string(&fx["commits"]).unwrap();
    for a in fx["assets"].as_array().unwrap() {
        let bytes = base64::engine::general_purpose::STANDARD.decode(a["base64"].as_str().unwrap()).unwrap();
        let staged = ws.stage_asset(who, &bytes).unwrap();
        text = text.replace(a["placeholder"].as_str().unwrap(), staged["object_id"].as_str().unwrap());
    }
    let commits: Vec<Value> = serde_json::from_str(&text).unwrap();
    for (i, c) in commits.iter().enumerate() {
        let req = json!({ "protocol_version": "1.0", "workspace_id": ws.workspace_id, "epoch": ws.epoch,
                          "idempotency_key": format!("fixture/{}", i + 1), "message": c["message"], "operations": c["operations"] });
        let r = ws.commit(&req, who, &CommitOpts::default());
        assert_eq!(r["status"], "accepted", "fixture commit {}: {r}", i + 1);
        assert_eq!(r["seq"], json!(i + 1));
    }
}

pub fn cell_rev(ws: &Workspace, who: &Caller, record: &str, field: &str) -> u64 {
    ws.read(who, "tasks", Some(&json!({ "kind": "table_cell", "record_id": record, "field_id": field }))).unwrap()["content"]["rev"]
        .as_u64()
        .unwrap()
}

pub fn set_cell(record: &str, field: &str, value: Value, rev: u64) -> Value {
    json!([{ "op": "table.set_values", "source_id": "tasks",
             "values": [{ "record_id": record, "field_id": field, "value": value, "expect": { "rev": rev } }] }])
}

/// The file a download (an `export` / `replica_bootstrap` result) names, read back from the named store.
pub fn fetched(objects: &ObjectStore, r: &Value) -> tempfile::TempPath {
    let path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    objects.read_to_file(r["download"]["obj_id"].as_str().unwrap(), &path).unwrap();
    path
}

/// `doc.query { sql }`.
pub fn sql(ws: &Workspace, who: &Caller, sql: &str) -> Result<Value, aiworkspace_core::WsError> {
    ws.query(who, &json!({ "sql": sql }))
}

/// The meta of an entity (`doc.read { as: "meta" }`).
pub fn meta(ws: &Workspace, who: &Caller, id: &str) -> Value {
    ws.read_as(who, &json!({ "entity_id": id, "as": "meta" })).unwrap()
}

/// The record ids a table view Block shows, in its order, through its compiled SQL (`meta.shows.view_sql`);
/// a view that no longer compiles gives its `view_error`.
pub fn view_ids(ws: &Workspace, who: &Caller, cell: &str) -> Result<Vec<String>, Value> {
    let m = meta(ws, who, cell);
    if let Some(e) = m["shows"].get("view_error") {
        return Err(e.clone());
    }
    let q = ws.query(who, &json!({ "sql": m["shows"]["view_sql"] })).unwrap();
    Ok(q["rows"].as_array().unwrap().iter().map(|r| r[0].as_str().unwrap().to_string()).collect())
}
