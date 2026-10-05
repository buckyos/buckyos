#![allow(dead_code)]
//! Shared helpers: a service on a temp folder with a controllable clock, and
//! the `project-workspace` fixture replayed through the service interface.

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
}

pub fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicI64::new(T0));
    let c = clock.clone();
    let svc = Service::open_with_clock(dir.path(), Arc::new(move || c.load(Ordering::SeqCst))).unwrap();
    Env { dir, svc, clock }
}

impl Env {
    pub fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }
    pub fn reopen(self) -> Env {
        let Env { dir, svc, clock } = self;
        drop(svc);
        let c = clock.clone();
        let svc = Service::open_with_clock(dir.path(), Arc::new(move || c.load(Ordering::SeqCst))).unwrap();
        Env { dir, svc, clock }
    }
}

pub fn alice() -> Caller {
    Caller::user("alice")
}
pub fn bob() -> Caller {
    Caller::user("bob")
}

static KEY: AtomicU64 = AtomicU64::new(0);

pub fn request(ws: &Workspace, ops: Value) -> Value {
    json!({ "protocol_version": "0.1", "workspace_id": ws.workspace_id, "epoch": ws.epoch,
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
        let req = json!({ "protocol_version": "0.1", "workspace_id": ws.workspace_id, "epoch": ws.epoch,
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
