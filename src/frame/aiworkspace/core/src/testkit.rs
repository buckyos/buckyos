//! A tiny in-memory Workspace used by this crate's tests and by the replica
//! prototype: plan → apply to `MemStore`, with a history for undo.

use crate::access::Access;
use crate::model::{MemStore, PROTOCOL_VERSION};
use crate::plan::{plan_commit, CommitFailure, CommitRequest, Limits, PlanEnv, PlannedOp};
use serde_json::{json, Value};

pub struct MemWorkspace {
    pub store: MemStore,
    pub head_seq: u64,
    pub history: Vec<Vec<PlannedOp>>,
    pub limits: Limits,
}

impl Default for MemWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl MemWorkspace {
    pub fn new() -> Self {
        MemWorkspace { store: MemStore::with_root(), head_seq: 0, history: vec![], limits: Limits::default() }
    }

    pub fn env(&self, principal: &str, origin: &str, run_id: Option<&str>, internal: bool) -> PlanEnv {
        PlanEnv {
            seq: self.head_seq + 1,
            principal: principal.to_string(),
            origin: origin.to_string(),
            run_id: run_id.map(str::to_string),
            now: "2026-10-04T08:00:00.000Z".into(),
            internal,
            import: false,
            replay: false,
            peer_id: 1,
            limits: self.limits.clone(),
        }
    }

    pub fn request(ops: Value) -> CommitRequest {
        CommitRequest::parse(&json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": "ws_test", "epoch": "ep_test",
            "idempotency_key": "k", "operations": ops }))
        .expect("request")
    }

    pub fn commit_with(&mut self, access: &Access, env: &PlanEnv, req: &CommitRequest) -> Result<u64, CommitFailure> {
        let plan = plan_commit(&self.store, req, env, access)?;
        let ops = plan.ops;
        let changes = plan.overlay.into_changes();
        self.store.apply(changes);
        self.head_seq = env.seq;
        self.history.push(ops);
        Ok(env.seq)
    }

    /// Commit as an all-powerful human `alice`.
    pub fn commit(&mut self, ops: Value) -> Result<u64, CommitFailure> {
        let env = self.env("alice", "human", None, false);
        self.commit_with(&Access::full("alice"), &env, &Self::request(ops))
    }

    pub fn ok(&mut self, ops: Value) -> u64 {
        match self.commit(ops) {
            Ok(s) => s,
            Err(f) => panic!("commit failed: {}", f.to_json()),
        }
    }

    pub fn fail(&mut self, ops: Value) -> Value {
        match self.commit(ops) {
            Ok(_) => panic!("commit unexpectedly accepted"),
            Err(f) => f.to_json(),
        }
    }

    /// Compensate the commit with sequence number `seq` (inverse ops, reversed).
    pub fn undo(&mut self, seq: u64) -> Result<u64, CommitFailure> {
        let mut inverse = Vec::new();
        for op in self.history[(seq - 1) as usize].iter().rev() {
            inverse.extend(op.inverse.clone().expect("compensable"));
        }
        let env = self.env("alice", "human", None, true);
        self.commit_with(&Access::full("alice"), &env, &Self::request(Value::Array(inverse)))
    }
}
