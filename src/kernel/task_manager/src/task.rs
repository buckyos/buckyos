use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Canceled,
    WaitingForApproval,
}

impl TaskStatus {
    pub fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "Pending" => Ok(TaskStatus::Pending),
            "Running" => Ok(TaskStatus::Running),
            "Paused" => Ok(TaskStatus::Paused),
            "Completed" => Ok(TaskStatus::Completed),
            "Failed" => Ok(TaskStatus::Failed),
            "Canceled" => Ok(TaskStatus::Canceled),
            "WaitingForApproval" => Ok(TaskStatus::WaitingForApproval),
            _ => Err(()),
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for TaskStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        TaskStatus::from_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Serialize, Deserialize)]
pub enum TaskScope {
    Private,
    User,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPermissions {
    pub read: TaskScope,
    pub write: TaskScope,
}

impl Default for TaskPermissions {
    fn default() -> Self {
        Self {
            read: TaskScope::User,
            write: TaskScope::Private,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub user_id: String,
    pub app_id: String,
    pub parent_id: Option<i64>,
    pub root_id: Option<i64>,
    pub name: String,
    pub task_type: String,
    pub status: TaskStatus,
    pub progress: f32,
    pub message: Option<String>,
    pub data: Value,
    pub permissions: TaskPermissions,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Task {
    pub fn new(
        name: String,
        task_type: String,
        user_id: String,
        app_id: String,
        parent_id: Option<i64>,
        permissions: TaskPermissions,
        data: Value,
    ) -> Self {
        let now = chrono::Utc::now().timestamp() as u64;
        Task {
            id: 0,
            user_id,
            app_id,
            parent_id,
            root_id: None,
            name,
            task_type,
            status: TaskStatus::Pending,
            progress: 0.0,
            message: None,
            data,
            permissions,
            created_at: now,
            updated_at: now,
        }
    }
}
