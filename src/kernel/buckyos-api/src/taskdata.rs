use crate::{FunctionObject, ThunkExecutionResult, ThunkObject};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<TaskDataRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<TaskDataResult>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl TaskData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn machine_request(payload: Value) -> Self {
        Self {
            request: Some(TaskDataRequest::Machine { payload }),
            ..Self::default()
        }
    }

    pub fn human_prompt(prompt: HumanTaskPrompt) -> Self {
        Self {
            request: Some(TaskDataRequest::Human { prompt }),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskDataRequest {
    Machine { payload: Value },
    Human { prompt: HumanTaskPrompt },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskDataResult {
    Machine {
        output: Value,
    },
    Human {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        values: BTreeMap<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        submitted_by: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        submitted_at: Option<i64>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDataProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<TaskDataCounter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<TaskDataCounter>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counters: BTreeMap<String, TaskDataCounter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

impl TaskDataProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_items(completed: u64, total: Option<u64>) -> Self {
        Self {
            items: Some(TaskDataCounter::new(completed, total)),
            ..Self::default()
        }
    }

    pub fn with_bytes(completed: u64, total: Option<u64>) -> Self {
        Self {
            bytes: Some(TaskDataCounter::new(completed, total)),
            ..Self::default()
        }
    }

    pub fn primary_percent(&self) -> Option<f32> {
        self.items
            .as_ref()
            .or(self.bytes.as_ref())
            .and_then(TaskDataCounter::percent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDataCounter {
    pub completed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl TaskDataCounter {
    pub fn new(completed: u64, total: Option<u64>) -> Self {
        Self { completed, total }
    }

    pub fn percent(&self) -> Option<f32> {
        let total = self.total?;
        if total == 0 {
            return None;
        }
        Some((self.completed as f32 / total as f32) * 100.0)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HumanTaskPrompt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<HumanPromptControl>,
}

impl HumanTaskPrompt {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn message_box(
        title: impl Into<String>,
        message: impl Into<String>,
        options: Vec<HumanPromptOption>,
    ) -> Self {
        Self {
            title: Some(title.into()),
            message: Some(message.into()),
            controls: vec![HumanPromptControl::Choice {
                id: "action".to_string(),
                label: None,
                options,
                multiple: false,
                required: true,
            }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HumanPromptControl {
    Choice {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<HumanPromptOption>,
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        required: bool,
    },
    TextInput {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<String>,
        #[serde(default)]
        multiline: bool,
        #[serde(default)]
        required: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanPromptOption {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<HumanPromptOptionStyle>,
}

impl HumanPromptOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: None,
            style: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanPromptOptionStyle {
    Default,
    Primary,
    Danger,
}

pub const TASK_DATA_TYPE_DOWNLOAD: &str = "download";
pub const TASK_DATA_TYPE_SCHEDULER_DISPATCH_THUNK: &str = "scheduler.dispatch_thunk";
pub const TASK_DATA_TYPE_WORKFLOW_RUN: &str = "workflow/run";
pub const TASK_DATA_TYPE_WORKFLOW_STEP: &str = "workflow/step";
pub const TASK_DATA_TYPE_WORKFLOW_MAP_SHARD: &str = "workflow/map_shard";
pub const TASK_DATA_TYPE_WORKFLOW_THUNK: &str = "workflow/thunk";
pub const TASK_DATA_TYPE_WORKFLOW_SCHEDULE: &str = "workflow/schedule";
pub const TASK_DATA_TYPE_WORKFLOW_SEND_MESSAGE: &str = "workflow.send_message";
pub const TASK_DATA_TYPE_AGENT_DELEGATE: &str = "agent.delegate";
pub const TASK_DATA_TYPE_HUMAN_INPUT: &str = "human.input";
pub const TASK_DATA_TYPE_OPENDAN_ASYNC_TOOL: &str = "opendan.async_tool";
pub const TASK_DATA_TYPE_AICC_COMPUTE: &str = "aicc.compute";
pub const TASK_DATA_TYPE_APP_INSTALL: &str = "app.install";
pub const TASK_DATA_TYPE_APP_UNINSTALL: &str = "app.uninstall";
pub const TASK_DATA_TYPE_APP_START: &str = "app.start";
pub const TASK_DATA_TYPE_APP_UPDATE: &str = "app.update";
pub const TASK_DATA_TYPE_APP_UPDATE_BATCH: &str = "app.update_batch";
pub const TASK_DATA_TYPE_SERVICE_RPC: &str = "workflow.execute_rpc";
pub const TASK_DATA_TYPE_WORKFLOW_RUN_TARGET: &str = "workflow.run";
pub const TASK_DATA_TYPE_TOOL_EXEC_BASH: &str = "tool.exec_bash";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskDataType {
    Download,
    SchedulerDispatchThunk,
    WorkflowRun,
    WorkflowStep,
    WorkflowMapShard,
    WorkflowThunk,
    WorkflowSchedule,
    WorkflowSendMessage,
    AgentDelegate,
    HumanInput,
    OpenDanAsyncTool,
    AiccCompute,
    AppInstall,
    AppUninstall,
    AppStart,
    AppUpdate,
    AppUpdateBatch,
    ServiceRpc,
    WorkflowRunTarget,
    ToolExecBash,
}

impl TaskDataType {
    /// Every declared type. Lets the built-in task-schema catalog be checked
    /// for coverage at test time instead of drifting silently.
    pub const ALL: &'static [TaskDataType] = &[
        Self::Download,
        Self::SchedulerDispatchThunk,
        Self::WorkflowRun,
        Self::WorkflowStep,
        Self::WorkflowMapShard,
        Self::WorkflowThunk,
        Self::WorkflowSchedule,
        Self::WorkflowSendMessage,
        Self::AgentDelegate,
        Self::HumanInput,
        Self::OpenDanAsyncTool,
        Self::AiccCompute,
        Self::AppInstall,
        Self::AppUninstall,
        Self::AppStart,
        Self::AppUpdate,
        Self::AppUpdateBatch,
        Self::ServiceRpc,
        Self::WorkflowRunTarget,
        Self::ToolExecBash,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Download => TASK_DATA_TYPE_DOWNLOAD,
            Self::SchedulerDispatchThunk => TASK_DATA_TYPE_SCHEDULER_DISPATCH_THUNK,
            Self::WorkflowRun => TASK_DATA_TYPE_WORKFLOW_RUN,
            Self::WorkflowStep => TASK_DATA_TYPE_WORKFLOW_STEP,
            Self::WorkflowMapShard => TASK_DATA_TYPE_WORKFLOW_MAP_SHARD,
            Self::WorkflowThunk => TASK_DATA_TYPE_WORKFLOW_THUNK,
            Self::WorkflowSchedule => TASK_DATA_TYPE_WORKFLOW_SCHEDULE,
            Self::WorkflowSendMessage => TASK_DATA_TYPE_WORKFLOW_SEND_MESSAGE,
            Self::AgentDelegate => TASK_DATA_TYPE_AGENT_DELEGATE,
            Self::HumanInput => TASK_DATA_TYPE_HUMAN_INPUT,
            Self::OpenDanAsyncTool => TASK_DATA_TYPE_OPENDAN_ASYNC_TOOL,
            Self::AiccCompute => TASK_DATA_TYPE_AICC_COMPUTE,
            Self::AppInstall => TASK_DATA_TYPE_APP_INSTALL,
            Self::AppUninstall => TASK_DATA_TYPE_APP_UNINSTALL,
            Self::AppStart => TASK_DATA_TYPE_APP_START,
            Self::AppUpdate => TASK_DATA_TYPE_APP_UPDATE,
            Self::AppUpdateBatch => TASK_DATA_TYPE_APP_UPDATE_BATCH,
            Self::ServiceRpc => TASK_DATA_TYPE_SERVICE_RPC,
            Self::WorkflowRunTarget => TASK_DATA_TYPE_WORKFLOW_RUN_TARGET,
            Self::ToolExecBash => TASK_DATA_TYPE_TOOL_EXEC_BASH,
        }
    }
}

impl fmt::Display for TaskDataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskDataType {
    type Err = TaskDataParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            TASK_DATA_TYPE_DOWNLOAD => Ok(Self::Download),
            TASK_DATA_TYPE_SCHEDULER_DISPATCH_THUNK => Ok(Self::SchedulerDispatchThunk),
            TASK_DATA_TYPE_WORKFLOW_RUN => Ok(Self::WorkflowRun),
            TASK_DATA_TYPE_WORKFLOW_STEP => Ok(Self::WorkflowStep),
            TASK_DATA_TYPE_WORKFLOW_MAP_SHARD => Ok(Self::WorkflowMapShard),
            TASK_DATA_TYPE_WORKFLOW_THUNK => Ok(Self::WorkflowThunk),
            TASK_DATA_TYPE_WORKFLOW_SCHEDULE => Ok(Self::WorkflowSchedule),
            TASK_DATA_TYPE_WORKFLOW_SEND_MESSAGE => Ok(Self::WorkflowSendMessage),
            TASK_DATA_TYPE_AGENT_DELEGATE => Ok(Self::AgentDelegate),
            TASK_DATA_TYPE_HUMAN_INPUT => Ok(Self::HumanInput),
            TASK_DATA_TYPE_OPENDAN_ASYNC_TOOL => Ok(Self::OpenDanAsyncTool),
            TASK_DATA_TYPE_AICC_COMPUTE => Ok(Self::AiccCompute),
            TASK_DATA_TYPE_APP_INSTALL => Ok(Self::AppInstall),
            TASK_DATA_TYPE_APP_UNINSTALL => Ok(Self::AppUninstall),
            TASK_DATA_TYPE_APP_START => Ok(Self::AppStart),
            TASK_DATA_TYPE_APP_UPDATE => Ok(Self::AppUpdate),
            TASK_DATA_TYPE_APP_UPDATE_BATCH => Ok(Self::AppUpdateBatch),
            TASK_DATA_TYPE_SERVICE_RPC => Ok(Self::ServiceRpc),
            TASK_DATA_TYPE_WORKFLOW_RUN_TARGET => Ok(Self::WorkflowRunTarget),
            TASK_DATA_TYPE_TOOL_EXEC_BASH => Ok(Self::ToolExecBash),
            _ => Err(TaskDataParseError::UnknownTaskDataType(value.to_string())),
        }
    }
}

impl TryFrom<&str> for TaskDataType {
    type Error = TaskDataParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl Serialize for TaskDataType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TaskDataType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDataParseError {
    UnknownTaskDataType(String),
    InvalidTaskData {
        task_data_type: String,
        message: String,
    },
}

impl fmt::Display for TaskDataParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTaskDataType(value) => {
                write!(f, "unknown task data type: {}", value)
            }
            Self::InvalidTaskData {
                task_data_type,
                message,
            } => {
                write!(f, "invalid task data for {}: {}", task_data_type, message)
            }
        }
    }
}

impl std::error::Error for TaskDataParseError {}

pub fn parse_typed_task_data(
    task_data_type: &str,
    data: Value,
) -> Result<TypedTaskData, TaskDataParseError> {
    let task_data_type = TaskDataType::from_str(task_data_type)?;
    TypedTaskData::parse(task_data_type, data)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "task_data_type", content = "data")]
pub enum TypedTaskData {
    #[serde(rename = "download")]
    Download(DownloadTaskData),
    #[serde(rename = "scheduler.dispatch_thunk")]
    SchedulerDispatchThunk(ThunkTaskData),
    #[serde(rename = "workflow/run")]
    WorkflowRun(WorkflowRunTaskData),
    #[serde(rename = "workflow/step")]
    WorkflowStep(WorkflowStepTaskData),
    #[serde(rename = "workflow/map_shard")]
    WorkflowMapShard(WorkflowMapShardTaskData),
    #[serde(rename = "workflow/thunk")]
    WorkflowThunk(ThunkTaskData),
    #[serde(rename = "workflow/schedule")]
    WorkflowSchedule(WorkflowScheduleTaskData),
    #[serde(rename = "workflow.send_message")]
    WorkflowSendMessage(SendMessageTaskData),
    #[serde(rename = "agent.delegate")]
    AgentDelegate(AgentDelegateTaskData),
    #[serde(rename = "human.input")]
    HumanInput(HumanInputTaskData),
    #[serde(rename = "opendan.async_tool")]
    OpenDanAsyncTool(OpenDanAsyncToolTaskData),
    #[serde(rename = "aicc.compute")]
    AiccCompute(AiccComputeTaskData),
    #[serde(rename = "app.install")]
    AppInstall(AppInstallTaskData),
    #[serde(rename = "app.uninstall")]
    AppUninstall(AppUninstallTaskData),
    #[serde(rename = "app.start")]
    AppStart(AppStartTaskData),
    #[serde(rename = "app.update")]
    AppUpdate(AppUpdateTaskData),
    #[serde(rename = "app.update_batch")]
    AppUpdateBatch(AppUpdateBatchTaskData),
    #[serde(rename = "service.rpc")]
    ServiceRpc(ServiceRpcTaskData),
    #[serde(rename = "workflow.run")]
    WorkflowRunTarget(WorkflowRunTargetTaskData),
    #[serde(rename = "tool.exec_bash")]
    ToolExecBash(ToolExecBashTaskData),
}

impl TypedTaskData {
    pub fn parse(task_data_type: TaskDataType, data: Value) -> Result<Self, TaskDataParseError> {
        match task_data_type {
            TaskDataType::Download => parse_data::<DownloadTaskData>(task_data_type, data.clone())
                .map(Self::Download)
                .or_else(|_| parse_download_legacy(data).map(Self::Download)),
            TaskDataType::SchedulerDispatchThunk => parse_data(task_data_type, data.clone())
                .map(Self::SchedulerDispatchThunk)
                .or_else(|_| parse_thunk_legacy(data).map(Self::SchedulerDispatchThunk)),
            TaskDataType::WorkflowRun => parse_data(task_data_type, data.clone())
                .map(Self::WorkflowRun)
                .or_else(|_| parse_workflow_run_legacy(data).map(Self::WorkflowRun)),
            TaskDataType::WorkflowStep => parse_data(task_data_type, data.clone())
                .map(Self::WorkflowStep)
                .or_else(|_| parse_workflow_step_legacy(data).map(Self::WorkflowStep)),
            TaskDataType::WorkflowMapShard => parse_data(task_data_type, data.clone())
                .map(Self::WorkflowMapShard)
                .or_else(|_| parse_workflow_map_shard_legacy(data).map(Self::WorkflowMapShard)),
            TaskDataType::WorkflowThunk => parse_data(task_data_type, data.clone())
                .map(Self::WorkflowThunk)
                .or_else(|_| parse_thunk_legacy(data).map(Self::WorkflowThunk)),
            TaskDataType::WorkflowSchedule => parse_data(task_data_type, data.clone())
                .map(Self::WorkflowSchedule)
                .or_else(|_| parse_workflow_schedule_legacy(data).map(Self::WorkflowSchedule)),
            TaskDataType::WorkflowSendMessage => parse_data(task_data_type, data.clone())
                .map(Self::WorkflowSendMessage)
                .or_else(|_| parse_send_message_legacy(data).map(Self::WorkflowSendMessage)),
            TaskDataType::AgentDelegate => parse_data(task_data_type, data.clone())
                .map(Self::AgentDelegate)
                .or_else(|_| parse_agent_delegate_legacy(data).map(Self::AgentDelegate)),
            TaskDataType::HumanInput => parse_data(task_data_type, data.clone())
                .map(Self::HumanInput)
                .or_else(|_| parse_human_input_legacy(data).map(Self::HumanInput)),
            TaskDataType::OpenDanAsyncTool => {
                parse_data::<OpenDanAsyncToolTaskData>(task_data_type, data.clone())
                    .map(Self::OpenDanAsyncTool)
                    .or_else(|_| {
                        Ok(Self::OpenDanAsyncTool(OpenDanAsyncToolTaskData {
                            request: data,
                            result: None,
                        }))
                    })
            }
            TaskDataType::AiccCompute => parse_data(task_data_type, data.clone())
                .map(Self::AiccCompute)
                .or_else(|_| parse_aicc_compute_legacy(data).map(Self::AiccCompute)),
            // beta 2.2 schema v3：app.install/app.update 不做旧 schema legacy parser。
            TaskDataType::AppInstall => parse_data(task_data_type, data).map(Self::AppInstall),
            TaskDataType::AppUninstall => {
                parse_data(task_data_type, data.clone()).map(Self::AppUninstall)
            }
            TaskDataType::AppStart => parse_data(task_data_type, data.clone()).map(Self::AppStart),
            TaskDataType::AppUpdate => parse_data(task_data_type, data).map(Self::AppUpdate),
            TaskDataType::AppUpdateBatch => {
                parse_data(task_data_type, data).map(Self::AppUpdateBatch)
            }
            TaskDataType::ServiceRpc => parse_data(task_data_type, data.clone())
                .map(Self::ServiceRpc)
                .or_else(|_| parse_service_rpc_legacy(data).map(Self::ServiceRpc)),
            TaskDataType::WorkflowRunTarget => parse_data(task_data_type, data.clone())
                .map(Self::WorkflowRunTarget)
                .or_else(|_| parse_workflow_run_target_legacy(data).map(Self::WorkflowRunTarget)),
            TaskDataType::ToolExecBash => parse_data(task_data_type, data).map(Self::ToolExecBash),
        }
    }

    pub fn task_data_type(&self) -> TaskDataType {
        match self {
            Self::Download(_) => TaskDataType::Download,
            Self::SchedulerDispatchThunk(_) => TaskDataType::SchedulerDispatchThunk,
            Self::WorkflowRun(_) => TaskDataType::WorkflowRun,
            Self::WorkflowStep(_) => TaskDataType::WorkflowStep,
            Self::WorkflowMapShard(_) => TaskDataType::WorkflowMapShard,
            Self::WorkflowThunk(_) => TaskDataType::WorkflowThunk,
            Self::WorkflowSchedule(_) => TaskDataType::WorkflowSchedule,
            Self::WorkflowSendMessage(_) => TaskDataType::WorkflowSendMessage,
            Self::AgentDelegate(_) => TaskDataType::AgentDelegate,
            Self::HumanInput(_) => TaskDataType::HumanInput,
            Self::OpenDanAsyncTool(_) => TaskDataType::OpenDanAsyncTool,
            Self::AiccCompute(_) => TaskDataType::AiccCompute,
            Self::AppInstall(_) => TaskDataType::AppInstall,
            Self::AppUninstall(_) => TaskDataType::AppUninstall,
            Self::AppStart(_) => TaskDataType::AppStart,
            Self::AppUpdate(_) => TaskDataType::AppUpdate,
            Self::AppUpdateBatch(_) => TaskDataType::AppUpdateBatch,
            Self::ServiceRpc(_) => TaskDataType::ServiceRpc,
            Self::WorkflowRunTarget(_) => TaskDataType::WorkflowRunTarget,
            Self::ToolExecBash(_) => TaskDataType::ToolExecBash,
        }
    }
}

fn parse_data<T: DeserializeOwned>(
    task_data_type: TaskDataType,
    data: Value,
) -> Result<T, TaskDataParseError> {
    serde_json::from_value(data).map_err(|err| TaskDataParseError::InvalidTaskData {
        task_data_type: task_data_type.as_str().to_string(),
        message: err.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadTaskData {
    pub request: DownloadTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<DownloadTaskResult>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl DownloadTaskData {
    pub fn new(
        download_url: impl Into<String>,
        objid: Option<String>,
        options: Option<DownloadTaskOptions>,
    ) -> Self {
        let download_url = download_url.into();
        Self {
            request: DownloadTaskRequest {
                download_url: Some(download_url.clone()),
                urls: vec![download_url],
                objid,
                resolved_objid: None,
                options,
            },
            progress: Some(TaskDataProgress::with_bytes(0, None)),
            result: Some(DownloadTaskResult {
                state: Some("pending".to_string()),
                ..Default::default()
            }),
            extra: BTreeMap::new(),
        }
    }

    pub fn primary_url(&self) -> Option<&str> {
        self.request
            .download_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.request
                    .urls
                    .iter()
                    .map(|value| value.trim())
                    .find(|value| !value.is_empty())
            })
    }

    pub fn urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        if let Some(download_url) = self
            .request
            .download_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            urls.push(download_url.to_string());
        }

        for url in &self.request.urls {
            let url = url.trim();
            if !url.is_empty() && !urls.iter().any(|existing| existing == url) {
                urls.push(url.to_string());
            }
        }
        urls
    }

    pub fn add_url(&mut self, download_url: impl Into<String>) -> bool {
        let download_url = download_url.into();
        if self.urls().iter().any(|url| url == &download_url) {
            return false;
        }
        self.request.urls.push(download_url);
        true
    }

    pub fn objid(&self) -> Option<&str> {
        self.request
            .objid
            .as_deref()
            .or(self.request.resolved_objid.as_deref())
    }

    pub fn set_state(&mut self, state: impl Into<String>, mode: Option<String>) {
        let result = self.result.get_or_insert_with(Default::default);
        result.state = Some(state.into());
        if let Some(mode) = mode {
            result.mode = Some(mode);
        }
    }

    pub fn set_byte_progress(&mut self, downloaded_bytes: u64, total_bytes: Option<u64>) {
        self.progress = Some(TaskDataProgress::with_bytes(downloaded_bytes, total_bytes));
        let result = self.result.get_or_insert_with(Default::default);
        result.downloaded_bytes = Some(downloaded_bytes);
        result.total_bytes = total_bytes;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DownloadTaskRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_objid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<DownloadTaskOptions>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DownloadTaskOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_remote_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj_id_in_host: Option<bool>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DownloadTaskResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stored_objects: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_pkg_total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_pkg_completed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_sub_pkg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkTaskData {
    pub request: ThunkTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ThunkExecutionResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<NodeExecutorTaskState>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThunkTaskRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thunk_obj_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thunk: Option<ThunkObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_object: Option<FunctionObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<ThunkDispatch>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThunkDispatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ThunkDispatchDetails>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThunkDispatchDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thunk_obj_id: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeExecutorTaskState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunTaskData {
    pub request: WorkflowRunTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<WorkflowRunTaskResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_action: Option<TaskHumanAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<TaskDataErrorInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunTaskRequest {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub workflow_id: String,
    #[serde(default)]
    pub workflow_name: String,
    #[serde(default)]
    pub plan_version: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunTaskResult {
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub summary: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskHumanAction {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskDataErrorInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStepTaskData {
    pub request: WorkflowStepTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_action: Option<TaskHumanAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<TaskDataErrorInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStepTaskRequest {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_obj_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stakeholders: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_human_since: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowMapShardTaskData {
    pub request: WorkflowMapShardTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<TaskDataErrorInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowMapShardTaskRequest {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub shard_index: u32,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowScheduleTaskData {
    pub request: WorkflowScheduleTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<WorkflowScheduleTaskResult>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowScheduleTaskRequest {
    #[serde(default)]
    pub schedule_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Value>,
    // 下面三项让 Task DB 成为 schedule 的无损唯一真相源（owner/policy/description
    // 不在 Task 列里，必须随 TaskData 落库，否则重启从 Task DB 重建会丢触发语义）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<WorkflowScheduleOwner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<WorkflowSchedulePolicy>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowScheduleOwner {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub app_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSchedulePolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub misfire: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel_runs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_up_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_sec: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowScheduleTaskResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fire_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_id: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendMessageTaskData {
    pub request: SendMessageTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SendMessageTaskRequest {
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<ScheduleTriggerContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScheduleTriggerContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_time: Option<i64>,
    #[serde(default)]
    pub manual: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDelegateTaskData {
    pub request: AgentDelegateTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<AgentDelegateProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<AgentDelegateTaskResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskDataErrorInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentDelegateTaskRequest {
    #[serde(default)]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Set when the task was created by accepting a Task Dispatch Center
    /// handoff: the immutable cross-owner link back to the DispatchRecord.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_id: Option<String>,
    /// The logical agent this delegate task targets. Replaces the legacy
    /// `progress.execution.runner` as the target-identity carrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_refs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_hints: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reason_messages: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<ScheduleTriggerContext>,
}

/// Operation string for the first Dispatch Target pilot.
pub const AGENT_DELEGATE_OPERATION_V1: &str = "agent.delegate/v1";

/// Wire input for the `agent.delegate/v1` dispatch operation. Deliberately
/// the *widest* dispatchable shape (the core intent may be natural
/// language); most operations should register closed, schema-checked
/// inputs instead. Identity (requested-by / on-behalf-of / workflow_ref)
/// never rides in here — it comes from the dispatcher's auth envelope.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentDelegateDispatchInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_refs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_hints: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentDelegateProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_line_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentDelegateTaskResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_behavior: Option<Value>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanInputTaskData {
    pub request: HumanInputTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<HumanInputTaskResult>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HumanInputTaskRequest {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_by: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HumanInputTaskResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenDanAsyncToolTaskData {
    pub request: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiccComputeTaskData {
    pub request: AiccComputeTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<AiccComputeProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<AiccComputeTaskResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiccComputeTaskRequest {
    #[serde(default)]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiccComputeProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiccComputeTaskResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_output: Option<Value>,
}

/// App 安装任务的可恢复 transaction 数据（beta 2.2 schema v4）。
/// request 保存原始 source/options/policy；中间态全部在 `state` 中，
/// 每个 Stage 成功后先完整写入 `Task.progress.transaction` 再进入下一 Stage。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInstallTaskData {
    pub schema_version: u32,
    pub request: AppInstallTaskRequest,
    #[serde(flatten)]
    pub state: crate::app_install::InstallTransactionState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInstallTaskRequest {
    pub source: crate::app_install::InstallSource,
    pub creator_user_id: String,
    pub creator_app_id: String,
    /// Installation owner; may differ from creator for audited admin on-behalf-of installs.
    pub owner_user_id: String,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_plan: Option<crate::app_install::InstallPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_plan_fingerprint: Option<String>,
    #[serde(default)]
    pub policy: crate::app_install::InstallPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppInstallDisplayProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<crate::app_install::InstallStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub updated_at: u64,
}

/// Task.progress is always this envelope. Updating display progress therefore
/// cannot overwrite the durable transaction snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInstallProgressEnvelope {
    pub schema_version: u32,
    pub transaction_revision: u64,
    pub transaction: AppInstallTaskData,
    pub display: AppInstallDisplayProgress,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInstallTerminalOutput {
    pub schema_version: u32,
    pub transaction_revision: u64,
    pub result: crate::app_install::InstallTaskResult,
    pub status: crate::app_install::AppInstallStatusSnapshot,
}

impl AppInstallTaskData {
    /// 全量镜像 patch：deep merge 后 transaction 与本结构严格一致
    /// （被清空的可选字段以 null 写入触发删除）。
    pub fn to_full_patch(&self) -> Value {
        let mut patch = self.state.to_full_patch();
        if let Some(map) = patch.as_object_mut() {
            map.insert(
                "schema_version".to_string(),
                Value::from(self.schema_version),
            );
            map.insert(
                "request".to_string(),
                serde_json::to_value(&self.request).unwrap_or(Value::Null),
            );
        }
        patch
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUninstallTaskData {
    pub schema_version: u32,
    pub request: AppUninstallTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<AppUninstallTaskResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUninstallTaskRequest {
    pub selector: String,
    pub app_instance_id: crate::AppInstanceId,
    pub creator_user_id: String,
    pub creator_app_id: String,
    pub idempotency_key: String,
    pub data_disposition: AppDataDisposition,
    pub deletion_manifest: AppDeletionManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppDataDisposition {
    Retain,
    Delete,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppDeletionManifest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cache_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUninstallTaskResult {
    pub app_instance_id: crate::AppInstanceId,
    pub data_disposition: AppDataDisposition,
    pub deleted_paths: Vec<String>,
    pub completed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppStartTaskData {
    pub schema_version: u32,
    pub request: AppStartTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<AppLifecycleTaskResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppStartTaskRequest {
    pub selector: String,
    pub app_instance_id: crate::AppInstanceId,
    pub creator_user_id: String,
    pub creator_app_id: String,
    pub idempotency_key: String,
    pub action: AppLifecycleAction,
    pub restart_strategy: RestartStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppLifecycleAction {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RestartStrategy {
    #[default]
    Recreate,
    Rolling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLifecycleTaskResult {
    pub app_instance_id: crate::AppInstanceId,
    pub action: AppLifecycleAction,
    pub desired_state: crate::ServiceState,
    pub ready_instance_count: u32,
    pub completed_at: u64,
}

/// App 升级任务的可恢复 transaction 数据（beta 2.2 schema v4）。
/// 与安装共用同一 Stage 流水线与中间态；旧 spec 回滚材料保存在
/// `state.prepared` 中的 scheduler 执行句柄。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateTaskData {
    pub schema_version: u32,
    pub request: AppUpdateTaskRequest,
    #[serde(flatten)]
    pub state: crate::app_install::InstallTransactionState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateTaskRequest {
    pub source: crate::app_install::InstallSource,
    pub creator_user_id: String,
    pub creator_app_id: String,
    pub owner_user_id: String,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_plan: Option<crate::app_install::InstallPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_plan_fingerprint: Option<String>,
    #[serde(default)]
    pub policy: crate::app_install::InstallPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
}

/// Catalog 批量升级 root task。输入冻结权威检查结果；runner 只为
/// `UpdateAvailable` 项创建 `app.update/v1` child task。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateBatchTaskData {
    pub schema_version: u32,
    pub request: AppUpdateBatchTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<AppUpdateBatchProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<AppUpdateBatchTaskResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppUpdateBatchTaskRequest {
    pub creator_user_id: String,
    pub creator_app_id: String,
    pub owner_user_id: String,
    pub idempotency_key: String,
    pub items: Vec<AppUpdateBatchRequestItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppUpdateBatchRequestItem {
    pub app_instance_id: crate::AppInstanceId,
    pub app_id: crate::AppId,
    pub source: crate::app_install::InstallSource,
    pub availability: crate::app_install::AppUpdateAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_plan: Option<crate::app_install::InstallPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_plan_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateBatchProgress {
    pub completed_items: u32,
    pub total_items: u32,
    pub items: Vec<AppUpdateBatchItemResult>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateBatchTaskResult {
    pub items: Vec<AppUpdateBatchItemResult>,
    pub succeeded: u32,
    pub failed: u32,
    pub satisfied: u32,
    pub blocked: u32,
    pub completed_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppUpdateBatchItemOutcome {
    Pending,
    Satisfied,
    Succeeded,
    Failed,
    Canceled,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateBatchItemResult {
    pub app_instance_id: crate::AppInstanceId,
    pub outcome: AppUpdateBatchItemOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AppUpdateTaskData {
    /// 同 `AppInstallTaskData::to_full_patch`。
    pub fn to_full_patch(&self) -> Value {
        let mut patch = self.state.to_full_patch();
        if let Some(map) = patch.as_object_mut() {
            map.insert(
                "schema_version".to_string(),
                Value::from(self.schema_version),
            );
            map.insert(
                "request".to_string(),
                serde_json::to_value(&self.request).unwrap_or(Value::Null),
            );
        }
        patch
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceRpcTaskData {
    pub request: ServiceRpcTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceRpcTaskRequest {
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<ScheduleTriggerContext>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunTargetTaskData {
    pub request: WorkflowRunTargetTaskRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunTargetTaskRequest {
    #[serde(default)]
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<ScheduleTriggerContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolExecBashTaskData {
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_wait: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_after: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_target: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyDownloadTaskData {
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    urls: Vec<String>,
    #[serde(default)]
    objid: Option<String>,
    #[serde(default)]
    resolved_objid: Option<String>,
    #[serde(default)]
    download_options: Option<DownloadTaskOptions>,
    #[serde(default)]
    download: Option<LegacyDownloadState>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyDownloadState {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    downloaded_bytes: Option<u64>,
    #[serde(default)]
    total_bytes: Option<u64>,
    #[serde(default)]
    local_path: Option<String>,
    #[serde(default)]
    result: Option<Value>,
}

fn parse_download_legacy(data: Value) -> Result<DownloadTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacyDownloadTaskData>(TaskDataType::Download, data)?;
    let progress = legacy
        .download
        .as_ref()
        .and_then(|download| {
            download
                .downloaded_bytes
                .map(|done| (done, download.total_bytes))
        })
        .map(|(done, total)| TaskDataProgress::with_bytes(done, total));
    let result = legacy.download.map(|download| DownloadTaskResult {
        state: download.state,
        mode: download.mode,
        local_path: download.local_path,
        downloaded_bytes: download.downloaded_bytes,
        total_bytes: download.total_bytes,
        chunk_count: None,
        stored_objects: Vec::new(),
        completed_at: None,
        sub_pkg_total: None,
        sub_pkg_completed: None,
        current_sub_pkg: None,
        output: download.result,
    });
    Ok(DownloadTaskData {
        request: DownloadTaskRequest {
            download_url: legacy.download_url,
            urls: legacy.urls,
            objid: legacy.objid,
            resolved_objid: legacy.resolved_objid,
            options: legacy.download_options,
        },
        progress,
        result,
        extra: legacy.extra,
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyThunkTaskData {
    #[serde(default)]
    runner: Option<String>,
    #[serde(default)]
    thunk_obj_id: Option<String>,
    #[serde(default)]
    thunk: Option<ThunkObject>,
    #[serde(default)]
    function_object: Option<FunctionObject>,
    #[serde(default)]
    dispatch: Option<ThunkDispatch>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    executor: Option<NodeExecutorTaskState>,
    #[serde(default)]
    executor_result: Option<ThunkExecutionResult>,
    #[serde(default)]
    workflow: Option<LegacyWorkflowThunk>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyWorkflowThunk {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    thunk_obj_id: Option<String>,
    #[serde(default)]
    attempt: Option<u32>,
    #[serde(default)]
    shard_index: Option<u32>,
}

fn parse_thunk_legacy(data: Value) -> Result<ThunkTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacyThunkTaskData>(TaskDataType::SchedulerDispatchThunk, data)?;
    let mut extra = legacy.extra;
    if let Some(workflow) = legacy.workflow {
        if let Some(value) = workflow.run_id {
            extra.insert("workflow_run_id".to_string(), Value::String(value));
        }
        if let Some(value) = workflow.attempt {
            extra.insert("workflow_attempt".to_string(), Value::from(value));
        }
        if let Some(value) = workflow.shard_index {
            extra.insert("workflow_shard_index".to_string(), Value::from(value));
        }
        let workflow_node_id = workflow.node_id;
        let workflow_thunk_obj_id = workflow.thunk_obj_id;
        Ok(ThunkTaskData {
            request: ThunkTaskRequest {
                runner: legacy.runner,
                node_id: legacy.node_id.or(workflow_node_id),
                thunk_obj_id: legacy.thunk_obj_id.or(workflow_thunk_obj_id),
                thunk: legacy.thunk,
                function_object: legacy.function_object,
                dispatch: legacy.dispatch,
                extra: BTreeMap::new(),
            },
            progress: None,
            result: legacy.executor_result,
            executor: legacy.executor,
            extra,
        })
    } else {
        Ok(ThunkTaskData {
            request: ThunkTaskRequest {
                runner: legacy.runner,
                node_id: legacy.node_id,
                thunk_obj_id: legacy.thunk_obj_id,
                thunk: legacy.thunk,
                function_object: legacy.function_object,
                dispatch: legacy.dispatch,
                extra: BTreeMap::new(),
            },
            progress: None,
            result: legacy.executor_result,
            executor: legacy.executor,
            extra,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyWorkflowRunTaskData {
    workflow: LegacyWorkflowRunFields,
    #[serde(default)]
    human_action: Option<TaskHumanAction>,
    #[serde(default)]
    last_error: Option<TaskDataErrorInfo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyWorkflowRunFields {
    #[serde(default)]
    run_id: String,
    #[serde(default)]
    workflow_id: String,
    #[serde(default)]
    workflow_name: String,
    #[serde(default)]
    plan_version: u32,
    #[serde(default)]
    status: String,
    #[serde(default)]
    summary: BTreeMap<String, u64>,
    #[serde(default)]
    updated_at: Option<i64>,
}

fn parse_workflow_run_legacy(data: Value) -> Result<WorkflowRunTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacyWorkflowRunTaskData>(TaskDataType::WorkflowRun, data)?;
    let total = legacy.workflow.summary.values().sum::<u64>();
    let completed = legacy
        .workflow
        .summary
        .get("Completed")
        .copied()
        .unwrap_or_default();
    Ok(WorkflowRunTaskData {
        request: WorkflowRunTaskRequest {
            run_id: legacy.workflow.run_id,
            workflow_id: legacy.workflow.workflow_id,
            workflow_name: legacy.workflow.workflow_name,
            plan_version: legacy.workflow.plan_version,
        },
        progress: (total > 0).then(|| TaskDataProgress::with_items(completed, Some(total))),
        result: Some(WorkflowRunTaskResult {
            status: legacy.workflow.status,
            summary: legacy.workflow.summary,
            updated_at: legacy.workflow.updated_at,
        }),
        human_action: legacy.human_action,
        last_error: legacy.last_error,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyWorkflowStepTaskData {
    workflow: WorkflowStepTaskRequest,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default)]
    human_action: Option<TaskHumanAction>,
    #[serde(default)]
    last_error: Option<TaskDataErrorInfo>,
}

fn parse_workflow_step_legacy(data: Value) -> Result<WorkflowStepTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacyWorkflowStepTaskData>(TaskDataType::WorkflowStep, data)?;
    Ok(WorkflowStepTaskData {
        request: legacy.workflow,
        progress: None,
        result: legacy.output,
        human_action: legacy.human_action,
        last_error: legacy.last_error,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyWorkflowMapShardTaskData {
    workflow: WorkflowMapShardTaskRequest,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default)]
    last_error: Option<TaskDataErrorInfo>,
}

fn parse_workflow_map_shard_legacy(
    data: Value,
) -> Result<WorkflowMapShardTaskData, TaskDataParseError> {
    let legacy =
        parse_data::<LegacyWorkflowMapShardTaskData>(TaskDataType::WorkflowMapShard, data)?;
    Ok(WorkflowMapShardTaskData {
        request: legacy.workflow,
        progress: None,
        result: legacy.output,
        last_error: legacy.last_error,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyWorkflowScheduleTaskData {
    schedule: LegacyWorkflowScheduleFields,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyWorkflowScheduleFields {
    #[serde(default)]
    schedule_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    schedule: Option<Value>,
    #[serde(default)]
    target: Option<Value>,
    #[serde(default)]
    next_fire_at: Option<i64>,
    #[serde(default)]
    last_fire_at: Option<i64>,
    #[serde(default)]
    last_task_id: Option<String>,
    #[serde(default)]
    last_run_id: Option<String>,
    #[serde(default)]
    consecutive_failures: u64,
    #[serde(default)]
    last_error: Option<Value>,
}

fn parse_workflow_schedule_legacy(
    data: Value,
) -> Result<WorkflowScheduleTaskData, TaskDataParseError> {
    let legacy =
        parse_data::<LegacyWorkflowScheduleTaskData>(TaskDataType::WorkflowSchedule, data)?;
    Ok(WorkflowScheduleTaskData {
        request: WorkflowScheduleTaskRequest {
            schedule_id: legacy.schedule.schedule_id,
            name: legacy.schedule.name,
            status: legacy.schedule.status,
            schedule: legacy.schedule.schedule,
            target: legacy.schedule.target,
            // 旧格式不带 owner/policy/description；留默认，由后续 update 回填。
            ..Default::default()
        },
        progress: None,
        result: Some(WorkflowScheduleTaskResult {
            next_fire_at: legacy.schedule.next_fire_at,
            last_fire_at: legacy.schedule.last_fire_at,
            last_task_id: legacy.schedule.last_task_id,
            last_run_id: legacy.schedule.last_run_id,
            consecutive_failures: legacy.schedule.consecutive_failures,
            last_error: legacy.schedule.last_error,
        }),
    })
}

#[derive(Debug, Clone, Deserialize)]
struct LegacySendMessageTaskData {
    send_message: SendMessageTaskRequest,
}

fn parse_send_message_legacy(data: Value) -> Result<SendMessageTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacySendMessageTaskData>(TaskDataType::WorkflowSendMessage, data)?;
    Ok(SendMessageTaskData {
        request: legacy.send_message,
        result: None,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyAgentDelegateTaskData {
    agent_delegate: LegacyAgentDelegateFields,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyAgentDelegateFields {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    requester_agent_id: Option<String>,
    #[serde(default)]
    owner_session_id: Option<String>,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default)]
    workspace_hints: Vec<Value>,
    #[serde(default)]
    reason_messages: Vec<Value>,
    #[serde(default)]
    trigger: Option<ScheduleTriggerContext>,
    #[serde(default)]
    route: Option<Value>,
    #[serde(default)]
    execution: Option<Value>,
    #[serde(default)]
    blocker: Option<Value>,
    #[serde(default)]
    human_input: Option<Value>,
    #[serde(default)]
    result: Option<AgentDelegateTaskResult>,
    #[serde(default)]
    error: Option<TaskDataErrorInfo>,
}

fn parse_agent_delegate_legacy(data: Value) -> Result<AgentDelegateTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacyAgentDelegateTaskData>(TaskDataType::AgentDelegate, data)?;
    let one_line_status = legacy
        .agent_delegate
        .execution
        .as_ref()
        .and_then(|value| value.get("one_line_status"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let updated_at_ms = legacy
        .agent_delegate
        .execution
        .as_ref()
        .and_then(|value| value.get("updated_at_ms"))
        .and_then(Value::as_i64);
    Ok(AgentDelegateTaskData {
        request: AgentDelegateTaskRequest {
            version: legacy.agent_delegate.version,
            source: legacy.agent_delegate.source,
            dispatch_id: None,
            target_agent_id: None,
            title: legacy.agent_delegate.title,
            purpose: legacy.agent_delegate.purpose,
            requester_agent_id: legacy.agent_delegate.requester_agent_id,
            owner_session_id: legacy.agent_delegate.owner_session_id,
            input: legacy.agent_delegate.input,
            context_refs: Vec::new(),
            workspace_hints: legacy.agent_delegate.workspace_hints,
            constraints: None,
            reason_messages: legacy.agent_delegate.reason_messages,
            trigger: legacy.agent_delegate.trigger,
        },
        progress: legacy
            .agent_delegate
            .execution
            .as_ref()
            .map(|_| AgentDelegateProgress {
                execution: legacy.agent_delegate.execution.clone(),
                one_line_status,
                updated_at_ms,
            }),
        result: legacy.agent_delegate.result,
        route: legacy.agent_delegate.route,
        blocker: legacy.agent_delegate.blocker,
        human_input: legacy.agent_delegate.human_input,
        error: legacy.agent_delegate.error,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyHumanInputTaskData {
    human_input: LegacyHumanInputFields,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyHumanInputFields {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    required_by: Option<Value>,
    #[serde(default)]
    candidates: Vec<Value>,
    #[serde(default)]
    response_schema: Option<Value>,
    #[serde(default)]
    response: Option<Value>,
    #[serde(default)]
    answered_by: Option<String>,
    #[serde(default)]
    answered_at: Option<i64>,
}

fn parse_human_input_legacy(data: Value) -> Result<HumanInputTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacyHumanInputTaskData>(TaskDataType::HumanInput, data)?;
    let result = legacy
        .human_input
        .response
        .as_ref()
        .filter(|value| !value.is_null())
        .map(|_| HumanInputTaskResult {
            response: legacy.human_input.response.clone(),
            answered_by: legacy.human_input.answered_by.clone(),
            answered_at: legacy.human_input.answered_at,
        });
    Ok(HumanInputTaskData {
        request: HumanInputTaskRequest {
            version: legacy.human_input.version,
            kind: legacy.human_input.kind,
            question: legacy.human_input.question,
            required_by: legacy.human_input.required_by,
            candidates: legacy.human_input.candidates,
            response_schema: legacy.human_input.response_schema,
        },
        result,
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyAiccComputeTaskData {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    owner_session_id: Option<String>,
    aicc: LegacyAiccComputeFields,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyAiccComputeFields {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    external_task_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    created_at_ms: Option<i64>,
    #[serde(default)]
    updated_at_ms: Option<i64>,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    event_ref: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    request: Option<Value>,
    #[serde(default)]
    provider_input: Option<Value>,
    #[serde(default)]
    route: Option<Value>,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default)]
    provider_output: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    events: Vec<Value>,
}

fn parse_aicc_compute_legacy(data: Value) -> Result<AiccComputeTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacyAiccComputeTaskData>(TaskDataType::AiccCompute, data)?;
    Ok(AiccComputeTaskData {
        request: AiccComputeTaskRequest {
            version: legacy.aicc.version,
            external_task_id: legacy.aicc.external_task_id,
            tenant_id: legacy.aicc.tenant_id,
            event_ref: legacy.aicc.event_ref,
            session_id: legacy.aicc.session_id.or(legacy.session_id),
            owner_session_id: legacy.owner_session_id,
            request: legacy.aicc.request,
            provider_input: legacy.aicc.provider_input,
            route: legacy.aicc.route,
            created_at_ms: legacy.aicc.created_at_ms,
        },
        progress: Some(AiccComputeProgress {
            status: legacy.aicc.status,
            updated_at_ms: legacy.aicc.updated_at_ms,
            events: legacy.aicc.events,
        }),
        result: Some(AiccComputeTaskResult {
            output: legacy.aicc.output,
            provider_output: legacy.aicc.provider_output,
        }),
        error: legacy.aicc.error,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyServiceRpcTaskData {
    service_rpc: ServiceRpcTaskRequest,
}

fn parse_service_rpc_legacy(data: Value) -> Result<ServiceRpcTaskData, TaskDataParseError> {
    let legacy = parse_data::<LegacyServiceRpcTaskData>(TaskDataType::ServiceRpc, data)?;
    Ok(ServiceRpcTaskData {
        request: legacy.service_rpc,
        result: None,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyWorkflowRunTargetTaskData {
    workflow_run: WorkflowRunTargetTaskRequest,
}

fn parse_workflow_run_target_legacy(
    data: Value,
) -> Result<WorkflowRunTargetTaskData, TaskDataParseError> {
    let legacy =
        parse_data::<LegacyWorkflowRunTargetTaskData>(TaskDataType::WorkflowRunTarget, data)?;
    Ok(WorkflowRunTargetTaskData {
        request: legacy.workflow_run,
        result: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_machine_task_data() {
        let mut data = TaskData::machine_request(json!({ "url": "https://example.test/file" }));
        data.progress = Some(TaskDataProgress::with_bytes(512, Some(1024)));
        data.result = Some(TaskDataResult::Machine {
            output: json!({ "path": "/tmp/file" }),
        });

        let value = serde_json::to_value(data).unwrap();

        assert_eq!(value["request"]["kind"], "machine");
        assert_eq!(value["progress"]["bytes"]["completed"], 512);
        assert_eq!(value["progress"]["bytes"]["total"], 1024);
        assert_eq!(value["result"]["output"]["path"], "/tmp/file");
    }

    #[test]
    fn serializes_human_prompt_and_result() {
        let prompt = HumanTaskPrompt::message_box(
            "Confirm",
            "Continue?",
            vec![
                HumanPromptOption::new("yes", "Yes"),
                HumanPromptOption::new("no", "No"),
            ],
        );
        let mut values = BTreeMap::new();
        values.insert("action".to_string(), json!("yes"));

        let data = TaskData {
            request: Some(TaskDataRequest::Human { prompt }),
            result: Some(TaskDataResult::Human {
                action: Some("yes".to_string()),
                values,
                submitted_by: Some("user".to_string()),
                submitted_at: Some(1730000000),
            }),
            ..TaskData::default()
        };

        let value = serde_json::to_value(data).unwrap();

        assert_eq!(value["request"]["kind"], "human");
        assert_eq!(value["request"]["prompt"]["controls"][0]["kind"], "choice");
        assert_eq!(value["result"]["kind"], "human");
        assert_eq!(value["result"]["values"]["action"], "yes");
    }

    #[test]
    fn calculates_progress_percent_only_when_total_is_known() {
        assert_eq!(TaskDataCounter::new(25, Some(100)).percent(), Some(25.0));
        assert_eq!(TaskDataCounter::new(25, None).percent(), None);
        assert_eq!(TaskDataCounter::new(25, Some(0)).percent(), None);
    }

    #[test]
    fn parses_download_legacy_schema_into_semantic_task_data() {
        let typed = parse_typed_task_data(
            TASK_DATA_TYPE_DOWNLOAD,
            json!({
                "download_url": "https://example.test/file.pkg",
                "urls": ["https://example.test/file.pkg"],
                "download_options": {
                    "filename": "file.pkg"
                },
                "download": {
                    "state": "running",
                    "mode": "local_file",
                    "downloaded_bytes": 512,
                    "total_bytes": 1024,
                    "local_path": "/tmp/file.pkg"
                }
            }),
        )
        .unwrap();

        let TypedTaskData::Download(data) = typed else {
            panic!("expected download task data");
        };

        assert_eq!(
            data.request.download_url.as_deref(),
            Some("https://example.test/file.pkg")
        );
        assert_eq!(
            data.progress
                .as_ref()
                .and_then(TaskDataProgress::primary_percent),
            Some(50.0)
        );
        assert_eq!(
            data.result
                .as_ref()
                .and_then(|result| result.state.as_deref()),
            Some("running")
        );
    }

    #[test]
    fn parses_human_input_legacy_schema_into_request_and_result() {
        let typed = parse_typed_task_data(
            TASK_DATA_TYPE_HUMAN_INPUT,
            json!({
                "human_input": {
                    "version": 1,
                    "kind": "agent_wait_user_msg",
                    "question": "Continue?",
                    "candidates": [],
                    "response_schema": { "type": "object" },
                    "response": { "answer": "yes" },
                    "answered_by": "user-a",
                    "answered_at": 1730000000
                }
            }),
        )
        .unwrap();

        let TypedTaskData::HumanInput(data) = typed else {
            panic!("expected human input task data");
        };

        assert_eq!(data.request.kind, "agent_wait_user_msg");
        assert_eq!(data.request.question.as_deref(), Some("Continue?"));
        assert_eq!(
            data.result.as_ref().unwrap().response,
            Some(json!({"answer": "yes"}))
        );
    }

    #[test]
    fn app_install_task_data_roundtrip_requires_schema_version() {
        // 当前 schema：request 保存原始 source/policy，事务中间态 flatten 在同级。
        let typed = parse_typed_task_data(
            TASK_DATA_TYPE_APP_INSTALL,
            json!({
                "schema_version": crate::app_install::APP_INSTALL_SCHEMA_VERSION,
                "request": {
                    "source": { "kind": "identifier", "identifier": "did:bns:demo.tester" },
                    "creator_user_id": "user",
                    "creator_app_id": "buckyos-tool",
                    "owner_user_id": "user",
                    "idempotency_key": "install-demo-1",
                    "policy": "NORMAL"
                },
                "stage": "resolve",
                "stage_revision": 1
            }),
        )
        .unwrap();

        let TypedTaskData::AppInstall(data) = typed else {
            panic!("expected app install task data");
        };

        assert_eq!(data.request.owner_user_id, "user");
        assert_eq!(
            data.schema_version,
            crate::app_install::APP_INSTALL_SCHEMA_VERSION
        );
        assert!(matches!(
            data.request.source,
            crate::app_install::InstallSource::Identifier { .. }
        ));
        assert_eq!(
            data.state.stage,
            Some(crate::app_install::InstallStage::Resolve)
        );
        assert_eq!(data.state.stage_revision, 1);

        // 旧 app_id/version schema 必须直接拒绝（beta2.2 breaking change，无 legacy parser）。
        let legacy = parse_typed_task_data(
            TASK_DATA_TYPE_APP_INSTALL,
            json!({
                "app_id": "demo",
                "user_id": "user",
                "version": "1.0.0",
                "content_id": "obj"
            }),
        );
        assert!(legacy.is_err());
    }

    #[test]
    fn rejects_unknown_task_data_type() {
        let err = parse_typed_task_data("unknown.type", json!({})).unwrap_err();
        assert!(matches!(err, TaskDataParseError::UnknownTaskDataType(_)));
    }
}
