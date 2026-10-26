//! The tasks of the run a session is driving now, for a Session process on
//! another machine (`session.task`, primary-runtime TODO §6.1): the same
//! answers the task bridge gives a local command, from the drive's task
//! resolver. A drive registers itself; an ended drive is gone with it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use agent_tool::task_bridge::{TaskBridgeRequest, TaskBridgeResponse};

use super::shared::Shared;

static LIVE: Mutex<Option<HashMap<String, Weak<Shared>>>> = Mutex::new(None);

/// The drive of session `sid` in this process is `sh`.
pub(super) fn register(sid: &str, sh: &Arc<Shared>) {
    let mut live = LIVE.lock().expect("live drives");
    let map = live.get_or_insert_with(HashMap::new);
    map.retain(|_, w| w.strong_count() > 0);
    map.insert(sid.to_string(), Arc::downgrade(sh));
}

/// Answer a task request of session `sid` from its current run; `None`:
/// this process drives no run of that session now.
pub async fn answer_session_task(sid: &str, req: &TaskBridgeRequest) -> Option<TaskBridgeResponse> {
    let sh = LIVE
        .lock()
        .expect("live drives")
        .as_ref()
        .and_then(|m| m.get(sid))
        .and_then(Weak::upgrade)?;
    let resolver = sh.tasks.lock().expect("tasks").clone()?;
    Some(agent_tool::task_bridge::answer(resolver.as_ref(), req).await)
}
