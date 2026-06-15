use anyhow::{bail, Context, Result};
use buckyos_api::{
    get_session_token_env_key, parse_typed_task_data, ClaimTaskResult, FunctionObject,
    FunctionType, NodeExecutorTaskState, Task, TaskFilter, TaskManagerClient, TaskStatus,
    ThunkExecutionResult, ThunkExecutionStatus, ThunkObject, ThunkTaskData, TypedTaskData,
    TASK_MANAGER_SERVICE_PORT,
};
use buckyos_kit::get_buckyos_service_data_dir;
use log::{info, warn};
use ndn_lib::ObjId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use shlex::Shlex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, Instant};

use ::kRPC::kRPC;

pub const NODE_EXECUTOR_TASK_TYPE: &str = "scheduler.dispatch_thunk";
pub const NODE_DAEMON_APP_ID: &str = "node-daemon";
const TASK_CLAIM_LEASE: Duration = Duration::from_secs(60);
const TASK_CLAIM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct NodeExecutorConfig {
    pub node_id: String,
    pub poll_interval: Duration,
    pub task_type: String,
    pub task_manager_url: String,
    pub session_token: Option<String>,
    pub work_root: PathBuf,
}

impl NodeExecutorConfig {
    pub fn new(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            poll_interval: Duration::from_secs(2),
            task_type: NODE_EXECUTOR_TASK_TYPE.to_string(),
            task_manager_url: format!(
                "http://127.0.0.1:{}/kapi/task-manager",
                TASK_MANAGER_SERVICE_PORT
            ),
            session_token: load_node_daemon_session_token(),
            work_root: get_buckyos_service_data_dir("node_daemon").join("node_executor"),
        }
    }
}

pub struct NodeExecutor {
    config: NodeExecutorConfig,
    task_manager: TaskManagerClient,
    running: Mutex<HashMap<String, RunningThunkExecution>>,
}

struct RunningThunkExecution {
    task_id: i64,
    thunk_obj_id: String,
    claim_token: Option<String>,
    last_heartbeat_at: Instant,
    child: Child,
    started_at: Instant,
    timeout: Option<Duration>,
    result_path: PathBuf,
    work_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeExecutorTaskPayload {
    #[serde(default)]
    runner: Option<String>,
    #[serde(default)]
    thunk_obj_id: Option<String>,
    thunk: ThunkObject,
    function_object: FunctionObject,
}

struct ExecutionPlan {
    program: String,
    args: Vec<String>,
}

impl NodeExecutor {
    pub fn new(config: NodeExecutorConfig) -> Self {
        let client = TaskManagerClient::new(kRPC::new(
            config.task_manager_url.as_str(),
            config.session_token.clone(),
        ));
        Self {
            config,
            task_manager: client,
            running: Mutex::new(HashMap::new()),
        }
    }

    pub async fn run_loop(&self) -> Result<()> {
        loop {
            self.sync_once().await?;
            sleep(self.config.poll_interval).await;
        }
    }

    pub async fn sync_once(&self) -> Result<usize> {
        self.reconcile_running_tasks().await?;
        self.start_pending_tasks().await
    }

    pub async fn run_task(&self, task_id: i64) -> Result<bool> {
        let task = self.task_manager.get_task(task_id).await?;
        if task.status == TaskStatus::Pending {
            let claim = self
                .task_manager
                .claim_task_with_lease(
                    task.id,
                    self.config.node_id.as_str(),
                    Some(TASK_CLAIM_LEASE),
                )
                .await?;
            if claim.task.is_none() {
                return Ok(false);
            }
            return self.maybe_start_task(claim).await;
        }
        self.start_task_execution(task, None).await
    }

    async fn start_pending_tasks(&self) -> Result<usize> {
        let tasks = self
            .task_manager
            .list_tasks(
                Some(TaskFilter {
                    task_type: Some(self.config.task_type.clone()),
                    runner: Some(self.config.node_id.clone()),
                    status: Some(TaskStatus::Pending),
                    ..Default::default()
                }),
                None,
                None,
            )
            .await
            .context("list pending thunk tasks failed")?;

        let mut launched = 0;
        for task in tasks {
            let claim = self
                .task_manager
                .claim_task_with_lease(
                    task.id,
                    self.config.node_id.as_str(),
                    Some(TASK_CLAIM_LEASE),
                )
                .await
                .with_context(|| format!("claim pending task {} failed", task.id))?;
            let Some(task) = claim.task.clone() else {
                continue;
            };
            match self.maybe_start_task(claim).await {
                Ok(true) => launched += 1,
                Ok(false) => {}
                Err(err) => {
                    warn!("node_executor failed to start task {}: {}", task.id, err);
                    let _ = self
                        .task_manager
                        .mark_task_as_failed(task.id, err.to_string().as_str())
                        .await;
                }
            }
        }

        Ok(launched)
    }

    async fn maybe_start_task(&self, claim: ClaimTaskResult) -> Result<bool> {
        let Some(task) = claim.task else {
            return Ok(false);
        };
        self.start_task_execution(task, claim.claim_token).await
    }

    async fn start_task_execution(&self, task: Task, claim_token: Option<String>) -> Result<bool> {
        let payload = NodeExecutorTaskPayload::from_task(&task)?;
        let thunk_obj_id = payload
            .thunk_obj_id
            .clone()
            .unwrap_or(calc_thunk_obj_id(&payload.thunk)?);

        if self.running.lock().await.contains_key(&thunk_obj_id) {
            bail!("thunk {} is already running on this node", thunk_obj_id);
        }

        let execution = self
            .spawn_task_execution(&task, &payload, &thunk_obj_id, claim_token)
            .await
            .with_context(|| format!("spawn task {} failed", task.id))?;
        self.mark_task_running(&task, &execution, &payload).await?;
        self.running.lock().await.insert(thunk_obj_id, execution);
        Ok(true)
    }

    async fn spawn_task_execution(
        &self,
        task: &Task,
        payload: &NodeExecutorTaskPayload,
        thunk_obj_id: &str,
        claim_token: Option<String>,
    ) -> Result<RunningThunkExecution> {
        tokio::fs::create_dir_all(&self.config.work_root).await?;
        let work_dir = self.config.work_root.join(format!(
            "task-{}-{}",
            task.id,
            sanitize_component(thunk_obj_id)
        ));
        tokio::fs::create_dir_all(&work_dir).await?;

        let thunk_json = serde_json::to_string(&payload.thunk)?;
        tokio::fs::write(work_dir.join("thunk.json"), thunk_json.as_bytes()).await?;
        tokio::fs::write(
            work_dir.join("function_object.json"),
            serde_json::to_vec_pretty(&payload.function_object)?,
        )
        .await?;

        let result_path = work_dir.join("executor_result.json");
        let stdout_path = work_dir.join("stdout.log");
        let stderr_path = work_dir.join("stderr.log");
        let plan = build_execution_plan(&payload.function_object, &work_dir).await?;

        let stdout_file = std::fs::File::create(&stdout_path)
            .with_context(|| format!("create stdout log {:?} failed", stdout_path))?;
        let stderr_file = std::fs::File::create(&stderr_path)
            .with_context(|| format!("create stderr log {:?} failed", stderr_path))?;

        let mut command = Command::new(&plan.program);
        command
            .args(&plan.args)
            .current_dir(&work_dir)
            .env("THIS_THUNK", thunk_json)
            .env("THIS_TASK_ID", task.id.to_string())
            .env("THIS_NODE_ID", &self.config.node_id)
            .env(
                "EXECUTOR_RESULT_PATH",
                result_path.to_string_lossy().to_string(),
            )
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));

        let child = command.spawn().with_context(|| {
            format!(
                "spawn executor failed: program={} args={:?}",
                plan.program, plan.args
            )
        })?;

        Ok(RunningThunkExecution {
            task_id: task.id,
            thunk_obj_id: thunk_obj_id.to_string(),
            claim_token,
            last_heartbeat_at: Instant::now(),
            child,
            started_at: Instant::now(),
            timeout: payload.function_object.timeout.map(Duration::from_secs),
            result_path,
            work_dir,
        })
    }

    async fn mark_task_running(
        &self,
        task: &Task,
        execution: &RunningThunkExecution,
        payload: &NodeExecutorTaskPayload,
    ) -> Result<()> {
        let mut next_data = parse_thunk_task_data(task)?;
        next_data.request.node_id = Some(self.config.node_id.clone());
        next_data.request.runner = payload.runner.clone();
        next_data.request.thunk_obj_id = Some(execution.thunk_obj_id.clone());
        next_data.executor = Some(NodeExecutorTaskState {
            status: Some("running".to_string()),
            task_id: Some(task.id),
            work_dir: Some(execution.work_dir.to_string_lossy().to_string()),
            result_path: Some(execution.result_path.to_string_lossy().to_string()),
        });
        self.task_manager
            .update_task_data(task.id, thunk_task_data_value(&next_data)?)
            .await?;
        self.task_manager
            .update_task_status(task.id, TaskStatus::Running)
            .await?;
        Ok(())
    }

    async fn reconcile_running_tasks(&self) -> Result<()> {
        let thunk_ids = {
            let running = self.running.lock().await;
            running.keys().cloned().collect::<Vec<_>>()
        };

        for thunk_obj_id in thunk_ids {
            let (task_id, claim_token, should_heartbeat) =
                match self.running.lock().await.get(&thunk_obj_id) {
                    Some(execution) => (
                        execution.task_id,
                        execution.claim_token.clone(),
                        execution.claim_token.is_some()
                            && execution.last_heartbeat_at.elapsed()
                                >= TASK_CLAIM_HEARTBEAT_INTERVAL,
                    ),
                    None => continue,
                };

            let task_status = self
                .task_manager
                .get_task(task_id)
                .await
                .map(|task| task.status)
                .unwrap_or(TaskStatus::Running);

            if task_status == TaskStatus::Pending && claim_token.is_some() {
                warn!(
                    "node_executor task {} lost its claim and was requeued; stopping local execution",
                    task_id
                );
                if let Some(mut execution) = self.running.lock().await.remove(&thunk_obj_id) {
                    let _ = execution.child.kill().await;
                }
                continue;
            }

            if task_status == TaskStatus::Canceled {
                if let Some(mut execution) = self.running.lock().await.remove(&thunk_obj_id) {
                    let _ = execution.child.kill().await;
                    self.finish_cancelled_execution(&execution).await?;
                }
                continue;
            }

            if should_heartbeat {
                if let Some(claim_token) = claim_token.as_deref() {
                    match self
                        .task_manager
                        .heartbeat_task_claim(task_id, claim_token, Some(TASK_CLAIM_LEASE))
                        .await
                    {
                        Ok(_) => {
                            if let Some(execution) =
                                self.running.lock().await.get_mut(&thunk_obj_id)
                            {
                                execution.last_heartbeat_at = Instant::now();
                            }
                        }
                        Err(err) => {
                            warn!(
                                "node_executor heartbeat failed for task {}: {}",
                                task_id, err
                            );
                        }
                    }
                }
            }

            let timed_out = match self.running.lock().await.get(&thunk_obj_id) {
                Some(execution) => execution
                    .timeout
                    .map(|timeout| execution.started_at.elapsed() > timeout)
                    .unwrap_or(false),
                None => continue,
            };
            if timed_out {
                if let Some(mut execution) = self.running.lock().await.remove(&thunk_obj_id) {
                    let _ = execution.child.kill().await;
                    self.finish_timeout_execution(&execution).await?;
                }
                continue;
            }

            let exit_status = {
                let mut running = self.running.lock().await;
                if let Some(execution) = running.get_mut(&thunk_obj_id) {
                    execution.child.try_wait()?
                } else {
                    None
                }
            };

            if let Some(exit_status) = exit_status {
                if let Some(execution) = self.running.lock().await.remove(&thunk_obj_id) {
                    self.finish_exited_execution(execution, exit_status.success())
                        .await?;
                }
            }
        }

        Ok(())
    }

    async fn finish_cancelled_execution(&self, execution: &RunningThunkExecution) -> Result<()> {
        let result = build_terminal_result(
            &execution.thunk_obj_id,
            execution.task_id,
            ThunkExecutionStatus::Cancelled,
            None,
            Some("task cancelled".to_string()),
        )?;
        self.apply_terminal_result(execution.task_id, &result)
            .await?;
        self.task_manager
            .update_task_status(execution.task_id, TaskStatus::Canceled)
            .await?;
        Ok(())
    }

    async fn finish_timeout_execution(&self, execution: &RunningThunkExecution) -> Result<()> {
        let result = build_terminal_result(
            &execution.thunk_obj_id,
            execution.task_id,
            ThunkExecutionStatus::Failed,
            None,
            Some("executor timeout".to_string()),
        )?;
        self.apply_terminal_result(execution.task_id, &result)
            .await?;
        self.task_manager
            .mark_task_as_failed(execution.task_id, "executor timeout")
            .await?;
        Ok(())
    }

    async fn finish_exited_execution(
        &self,
        execution: RunningThunkExecution,
        exit_success: bool,
    ) -> Result<()> {
        let result = load_or_build_result(
            &execution.result_path,
            &execution.thunk_obj_id,
            execution.task_id,
            exit_success,
        )
        .await?;
        self.apply_terminal_result(execution.task_id, &result)
            .await?;

        match result.status {
            ThunkExecutionStatus::Success => {
                self.task_manager.complete_task(execution.task_id).await?;
            }
            ThunkExecutionStatus::Cancelled => {
                self.task_manager
                    .update_task_status(execution.task_id, TaskStatus::Canceled)
                    .await?;
            }
            ThunkExecutionStatus::Waiting | ThunkExecutionStatus::Dispatched => {
                self.task_manager
                    .mark_task_as_failed(
                        execution.task_id,
                        "executor returned non-terminal thunk status",
                    )
                    .await?;
            }
            ThunkExecutionStatus::Failed => {
                let error_message = result
                    .error
                    .as_deref()
                    .unwrap_or("executor returned failed result");
                self.task_manager
                    .mark_task_as_failed(execution.task_id, error_message)
                    .await?;
            }
        }

        Ok(())
    }

    async fn apply_terminal_result(
        &self,
        task_id: i64,
        result: &ThunkExecutionResult,
    ) -> Result<()> {
        let task = self.task_manager.get_task(task_id).await?;
        let mut next_data = parse_thunk_task_data(&task)?;
        next_data.request.thunk_obj_id = Some(result.thunk_obj_id.to_string());
        next_data.executor = Some(NodeExecutorTaskState {
            status: Some("finished".to_string()),
            task_id: Some(task_id),
            work_dir: next_data
                .executor
                .as_ref()
                .and_then(|executor| executor.work_dir.clone()),
            result_path: next_data
                .executor
                .as_ref()
                .and_then(|executor| executor.result_path.clone()),
        });
        next_data.result = Some(result.clone());
        self.task_manager
            .update_task_data(task_id, thunk_task_data_value(&next_data)?)
            .await?;
        Ok(())
    }
}

impl NodeExecutorTaskPayload {
    fn from_task(task: &Task) -> Result<Self> {
        let data = parse_thunk_task_data(task)?;
        let thunk = data
            .request
            .thunk
            .ok_or_else(|| anyhow::anyhow!("task.data.request.thunk is required"))?;
        let function_object = data
            .request
            .function_object
            .ok_or_else(|| anyhow::anyhow!("task.data.request.function_object is required"))?;
        let runner = data.request.runner.or_else(|| {
            data.request
                .dispatch
                .as_ref()
                .and_then(|dispatch| dispatch.runner.clone())
        });
        let thunk_obj_id = data.request.thunk_obj_id.or_else(|| {
            data.request
                .dispatch
                .and_then(|dispatch| dispatch.details)
                .and_then(|details| details.thunk_obj_id)
        });

        Ok(Self {
            runner,
            thunk_obj_id,
            thunk,
            function_object,
        })
    }
}

async fn build_execution_plan(
    function_object: &FunctionObject,
    work_dir: &Path,
) -> Result<ExecutionPlan> {
    match &function_object.func_type {
        FunctionType::ExecPkg => {
            let tokens = Shlex::new(function_object.content.as_str()).collect::<Vec<_>>();
            let Some((program, args)) = tokens.split_first() else {
                bail!("exec pkg content is empty");
            };
            Ok(ExecutionPlan {
                program: program.to_string(),
                args: args.to_vec(),
            })
        }
        FunctionType::Script(language) => {
            let interpreter = script_interpreter(language);
            let script_path =
                work_dir.join(format!("runner_script.{}", script_extension(language)));
            tokio::fs::write(&script_path, function_object.content.as_bytes()).await?;
            Ok(ExecutionPlan {
                program: interpreter.to_string(),
                args: vec![script_path.to_string_lossy().to_string()],
            })
        }
        FunctionType::OPTask(language) => {
            let interpreter = script_interpreter(language);
            let script_path = work_dir.join(format!("op_task.{}", script_extension(language)));
            tokio::fs::write(&script_path, function_object.content.as_bytes()).await?;
            Ok(ExecutionPlan {
                program: interpreter.to_string(),
                args: vec![script_path.to_string_lossy().to_string()],
            })
        }
        FunctionType::Operator => {
            bail!("operator execution is not supported by node_Executor yet")
        }
    }
}

async fn load_or_build_result(
    result_path: &Path,
    thunk_obj_id: &str,
    task_id: i64,
    exit_success: bool,
) -> Result<ThunkExecutionResult> {
    let mut result = if result_path.exists() {
        let content = tokio::fs::read_to_string(result_path)
            .await
            .with_context(|| format!("read executor result {:?} failed", result_path))?;
        serde_json::from_str::<ThunkExecutionResult>(&content)
            .with_context(|| format!("parse executor result {:?} failed", result_path))?
    } else if exit_success {
        build_terminal_result(
            thunk_obj_id,
            task_id,
            ThunkExecutionStatus::Success,
            None,
            None,
        )?
    } else {
        build_terminal_result(
            thunk_obj_id,
            task_id,
            ThunkExecutionStatus::Failed,
            None,
            Some("executor exited without result".to_string()),
        )?
    };

    result.task_id = task_id.to_string();
    result.thunk_obj_id = ObjId::new(thunk_obj_id)
        .with_context(|| format!("invalid thunk obj id {}", thunk_obj_id))?;

    if !exit_success && result.status == ThunkExecutionStatus::Success {
        result.status = ThunkExecutionStatus::Failed;
        result.error = Some("executor exited with non-zero status".to_string());
    }

    Ok(result)
}

fn build_terminal_result(
    thunk_obj_id: &str,
    task_id: i64,
    status: ThunkExecutionStatus,
    result: Option<Value>,
    error: Option<String>,
) -> Result<ThunkExecutionResult> {
    Ok(ThunkExecutionResult {
        thunk_obj_id: ObjId::new(thunk_obj_id)
            .with_context(|| format!("invalid thunk obj id {}", thunk_obj_id))?,
        task_id: task_id.to_string(),
        status,
        result_obj_id: None,
        result,
        result_url: None,
        error,
        metrics: Value::Null,
    })
}

fn parse_thunk_task_data(task: &Task) -> Result<ThunkTaskData> {
    match parse_typed_task_data(task.task_type.as_str(), task.data.clone())
        .with_context(|| format!("parse task {} thunk data failed", task.id))?
    {
        TypedTaskData::SchedulerDispatchThunk(data) | TypedTaskData::WorkflowThunk(data) => {
            Ok(data)
        }
        other => bail!(
            "task {} data type {} is not thunk-compatible",
            task.id,
            other.task_data_type()
        ),
    }
}

fn thunk_task_data_value(data: &ThunkTaskData) -> Result<Value> {
    serde_json::to_value(data).context("serialize thunk task data failed")
}

fn script_interpreter(language: &str) -> &'static str {
    match language.to_ascii_lowercase().as_str() {
        "python" | "python3" | "py" => "python3",
        "bash" | "sh" | "shell" => "bash",
        "javascript" | "js" | "node" => "node",
        _ => "bash",
    }
}

fn script_extension(language: &str) -> &'static str {
    match language.to_ascii_lowercase().as_str() {
        "python" | "python3" | "py" => "py",
        "javascript" | "js" | "node" => "js",
        _ => "sh",
    }
}

fn sanitize_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}

fn load_node_daemon_session_token() -> Option<String> {
    let key = get_session_token_env_key(NODE_DAEMON_APP_ID, false);
    std::env::var(&key).ok()
}

fn calc_thunk_obj_id(thunk: &ThunkObject) -> Result<String> {
    let bytes = serde_json::to_vec(thunk)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("thunk:{}", hex::encode(hasher.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AffinityType, FunctionParamType, FunctionResultType, TaskPermissions, TaskScope, TaskStatus,
    };
    use serde_json::json;

    fn sample_task(data: Value) -> Task {
        Task {
            id: 7,
            user_id: "root".to_string(),
            app_id: "scheduler".to_string(),
            session_id: String::new(),
            parent_id: None,
            root_id: "7".to_string(),
            name: "dispatch".to_string(),
            task_type: NODE_EXECUTOR_TASK_TYPE.to_string(),
            runner: String::new(),
            status: TaskStatus::Pending,
            progress: 0.0,
            message: None,
            data,
            permissions: TaskPermissions {
                read: TaskScope::System,
                write: TaskScope::System,
            },
            created_at: 0,
            updated_at: 0,
        }
    }

    fn sample_function_object(func_type: FunctionType) -> FunctionObject {
        FunctionObject {
            func_type,
            content: "echo test".to_string(),
            is_pure: true,
            timeout: Some(10),
            requirements: HashMap::new(),
            best_run_weight: HashMap::new(),
            affinity_type: AffinityType::Input,
            params_type: HashMap::from([(
                "input".to_string(),
                FunctionParamType::Fixed("string".to_string()),
            )]),
            result_type: FunctionResultType::Fixed("string".to_string()),
        }
    }

    fn sample_thunk() -> ThunkObject {
        ThunkObject {
            fun_id: ObjId::new("func:1234567890").unwrap(),
            params: HashMap::from([("input".to_string(), json!("hello"))]),
            metadata: json!({"run_id": "run-1"}),
        }
    }

    #[test]
    fn parses_payload_from_task_data() {
        let task = sample_task(json!({
            "node_id": "node-1",
            "runner": "package-runner",
            "thunk": sample_thunk(),
            "function_object": sample_function_object(FunctionType::ExecPkg),
        }));

        let payload = NodeExecutorTaskPayload::from_task(&task).unwrap();
        assert_eq!(payload.runner.as_deref(), Some("package-runner"));
    }

    #[test]
    fn parses_payload_with_nested_dispatch_receipt() {
        let task = sample_task(json!({
            "thunk": sample_thunk(),
            "function_object": sample_function_object(FunctionType::ExecPkg),
            "dispatch": {
                "node_id": "node-2",
                "runner": "script-runner:python",
                "details": {
                    "thunk_obj_id": "thunk:abcdef"
                }
            }
        }));

        let payload = NodeExecutorTaskPayload::from_task(&task).unwrap();
        assert_eq!(payload.runner.as_deref(), Some("script-runner:python"));
        assert_eq!(payload.thunk_obj_id.as_deref(), Some("thunk:abcdef"));
    }

    #[test]
    fn parses_payload_without_node_id() {
        let mut task = sample_task(json!({
            "thunk": sample_thunk(),
            "function_object": sample_function_object(FunctionType::ExecPkg),
        }));
        task.runner = "node-from-task-runner".to_string();

        let payload = NodeExecutorTaskPayload::from_task(&task).unwrap();
        assert_eq!(payload.runner, None);
    }

    #[tokio::test]
    async fn builds_script_execution_plan() {
        let unique = format!(
            "node-executor-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let work_dir = std::env::temp_dir().join(unique);
        tokio::fs::create_dir_all(&work_dir).await.unwrap();

        let mut function_object =
            sample_function_object(FunctionType::Script("python".to_string()));
        function_object.content = "print('ok')".to_string();
        let plan = build_execution_plan(&function_object, &work_dir)
            .await
            .unwrap();

        assert_eq!(plan.program, "python3");
        assert_eq!(plan.args.len(), 1);
        assert!(plan.args[0].ends_with(".py"));
    }
}
