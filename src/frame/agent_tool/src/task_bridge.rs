//! Task CLI bridge: the task tools (`wait_task` / `get_task_state` /
//! `cancel_task`) as a command run through `shell`
//! (`notepads/llm-context-chat-prompt-prereq-todo.md` P3).
//!
//! A run that offers `group:bash_core` with a task command
//! (`XllmDeps.task_cli`) serves its task resolver on a Unix socket and
//! hands the socket's name to every command through
//! [`ENV_TASK_ENDPOINT`]. The command (`agent-session task …` in an Agent
//! Session) sends one request line and prints the answer: the very text
//! the task tools return (`task_state_observation`), from the same
//! resolver, with the same wait limit.
//!
//! The socket lives as long as the run's tool manager. On Linux it is an
//! abstract socket (no file, no path length limit); elsewhere a socket file
//! in the temp directory. Only processes on the executor's host (and in its
//! network namespace) reach it: a remote runtime cannot follow tasks this
//! way.

use std::sync::Arc;

use llm_context::observation::Observation;
use llm_context::tasks::{
    task_state_observation_via, RunningTaskResolver, TaskFollowup, TaskState, MAX_IN_TOOL_WAIT_MS,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Environment variable naming the task bridge of the run a command runs
/// in.
pub const ENV_TASK_ENDPOINT: &str = "XLLM_TASK_ENDPOINT";

/// Default wait of `wait` when none is given: below the shell's default
/// wait (30s), so the command returns before it becomes a task itself.
pub const DEFAULT_CLI_WAIT_MS: u64 = 25_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOp {
    State,
    Wait,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBridgeRequest {
    pub op: TaskOp,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
}

/// `state`: `running | finished | failed | unknown | not_cancellable`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBridgeResponse {
    pub state: String,
    pub text: String,
}

impl TaskBridgeResponse {
    /// Exit code of the command: 0 running / finished, 1 failed or not
    /// cancellable, 2 unknown.
    pub fn exit_code(&self) -> i32 {
        match self.state.as_str() {
            "running" | "finished" => 0,
            "unknown" => 2,
            _ => 1,
        }
    }
}

fn response(task_id: &str, state: &TaskState, followup: &TaskFollowup) -> TaskBridgeResponse {
    let name = match state {
        TaskState::Running { .. } => "running",
        TaskState::Finished(r) if r.success => "finished",
        TaskState::Finished(_) => "failed",
        TaskState::Unknown { .. } => "unknown",
    };
    let text = match task_state_observation_via("", task_id, state, followup) {
        Observation::Success { content, .. } => match content {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        },
        Observation::Error { message, .. } => message,
        other => format!("{other:?}"),
    };
    TaskBridgeResponse {
        state: name.to_string(),
        text,
    }
}

/// Answer one request from `resolver` (the semantics of the task tools).
pub async fn answer(
    resolver: &dyn RunningTaskResolver,
    req: &TaskBridgeRequest,
) -> TaskBridgeResponse {
    let followup = resolver.followup();
    let id = req.task_id.trim();
    if id.is_empty() {
        return TaskBridgeResponse {
            state: "unknown".into(),
            text: "a task id is required".into(),
        };
    }
    resolver.watch(id);
    match req.op {
        TaskOp::State => response(id, &resolver.state(id).await, &followup),
        TaskOp::Wait => {
            let wait = req
                .wait_ms
                .filter(|n| *n > 0)
                .unwrap_or(DEFAULT_CLI_WAIT_MS)
                .min(MAX_IN_TOOL_WAIT_MS);
            let until = crate::now_ms() + wait;
            response(id, &resolver.wait(id, Some(until)).await, &followup)
        }
        TaskOp::Cancel => match resolver.cancel(id).await {
            Ok(state) => response(id, &state, &followup),
            Err(_) => TaskBridgeResponse {
                state: "not_cancellable".into(),
                text: format!("task {id} does not support cancellation; it keeps running"),
            },
        },
    }
}

/// A served bridge; dropping it stops serving.
pub struct TaskBridge {
    endpoint: String,
    task: tokio::task::JoinHandle<()>,
    #[cfg(not(target_os = "linux"))]
    path: std::path::PathBuf,
}

impl std::fmt::Debug for TaskBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskBridge")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl TaskBridge {
    /// The value of [`ENV_TASK_ENDPOINT`] for commands of this run.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl Drop for TaskBridge {
    fn drop(&mut self) {
        self.task.abort();
        #[cfg(not(target_os = "linux"))]
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn unique_name() -> String {
    format!(
        "xllm-task-{}-{}-{}",
        std::process::id(),
        crate::now_ms(),
        crate::tasks::next_local_seq()
    )
}

#[cfg(unix)]
fn bind(name: &str) -> std::io::Result<(std::os::unix::net::UnixListener, String)> {
    #[cfg(target_os = "linux")]
    {
        use std::os::linux::net::SocketAddrExt;
        let addr = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes())?;
        let listener = std::os::unix::net::UnixListener::bind_addr(&addr)?;
        Ok((listener, format!("@{name}")))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let path = std::env::temp_dir().join(format!("{name}.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        Ok((listener, path.display().to_string()))
    }
}

/// Serve `resolver` until the returned bridge is dropped.
#[cfg(unix)]
pub fn serve(resolver: Arc<dyn RunningTaskResolver>) -> std::io::Result<TaskBridge> {
    let name = unique_name();
    let (std_listener, endpoint) = bind(&name)?;
    std_listener.set_nonblocking(true)?;
    let listener = tokio::net::UnixListener::from_std(std_listener)?;
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let resolver = resolver.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut line = String::new();
                if BufReader::new(read).read_line(&mut line).await.is_err() {
                    return;
                }
                let resp = match serde_json::from_str::<TaskBridgeRequest>(line.trim()) {
                    Ok(req) => answer(resolver.as_ref(), &req).await,
                    Err(e) => TaskBridgeResponse {
                        state: "unknown".into(),
                        text: format!("bad request: {e}"),
                    },
                };
                let mut out = serde_json::to_string(&resp).unwrap_or_default();
                out.push('\n');
                let _ = write.write_all(out.as_bytes()).await;
            });
        }
    });
    Ok(TaskBridge {
        task,
        #[cfg(not(target_os = "linux"))]
        path: std::path::PathBuf::from(&endpoint),
        endpoint,
    })
}

#[cfg(not(unix))]
pub fn serve(_resolver: Arc<dyn RunningTaskResolver>) -> std::io::Result<TaskBridge> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the task bridge needs Unix sockets",
    ))
}

/// Send one request to the bridge at `endpoint` (the value of
/// [`ENV_TASK_ENDPOINT`]).
#[cfg(unix)]
pub async fn call(endpoint: &str, req: &TaskBridgeRequest) -> Result<TaskBridgeResponse, String> {
    let stream = match endpoint.strip_prefix('@') {
        #[cfg(target_os = "linux")]
        Some(name) => {
            use std::os::linux::net::SocketAddrExt;
            let addr = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes())
                .map_err(|e| format!("task endpoint {endpoint}: {e}"))?;
            let s = std::os::unix::net::UnixStream::connect_addr(&addr)
                .map_err(|e| format!("task endpoint {endpoint} not reachable: {e}"))?;
            s.set_nonblocking(true).map_err(|e| e.to_string())?;
            tokio::net::UnixStream::from_std(s).map_err(|e| e.to_string())?
        }
        _ => tokio::net::UnixStream::connect(endpoint)
            .await
            .map_err(|e| format!("task endpoint {endpoint} not reachable: {e}"))?,
    };
    let (read, mut write) = stream.into_split();
    let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    line.push('\n');
    write
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("task endpoint {endpoint}: {e}"))?;
    let mut answer = String::new();
    BufReader::new(read)
        .read_line(&mut answer)
        .await
        .map_err(|e| format!("task endpoint {endpoint}: {e}"))?;
    serde_json::from_str(answer.trim()).map_err(|e| format!("task endpoint {endpoint}: {e}"))
}

#[cfg(not(unix))]
pub async fn call(_endpoint: &str, _req: &TaskBridgeRequest) -> Result<TaskBridgeResponse, String> {
    Err("the task bridge needs Unix sockets".into())
}
