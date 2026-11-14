//! `ws.work_log { before?, limit?, filter? }` (UI 框架 PRD §4.5, K3): the store reads the timeline from the Commits,
//! the runs, the permission record and the session registry (`aiworkspace_store::worklog`); the server adds the
//! non-public shows it keeps in memory.

use crate::AppState;
use aiworkspace_core::{WsError, WsResult};
use aiworkspace_store::worklog::Query;
use aiworkspace_store::Caller;
use serde_json::Value;
use std::sync::Arc;

pub fn handle(state: &Arc<AppState>, p: &Value, caller: &Caller) -> WsResult<Value> {
    let ws_id = p.get("workspace_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("workspace_id required"))?;
    let q = Query::parse(p)?;
    state.svc.work_log(caller, ws_id, &q, state.shows.events(ws_id))
}
