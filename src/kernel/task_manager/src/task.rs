#[allow(dead_code)]
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TaskStatus {
    Pending,    // 任务已创建但尚未开始
    Running,    // 任务正在运行
    Paused,     // 任务已暂停
    Completed,  // 任务已完成
    Failed,     // 任务失败
    WaitingForApproval, // 任务完成但等待审核/批准
}

impl TaskStatus {
    pub fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "Pending" => Ok(TaskStatus::Pending),
            "Running" => Ok(TaskStatus::Running),
            "Paused" => Ok(TaskStatus::Paused),
            "Completed" => Ok(TaskStatus::Completed),
            "Failed" => Ok(TaskStatus::Failed),
            "WaitingForApproval" => Ok(TaskStatus::WaitingForApproval),
            _ => Err(()),
        }
    }
}
impl ToString for TaskStatus {
    fn to_string(&self) -> String {
        match *self {
            TaskStatus::Pending => "Pending".to_string(),
            TaskStatus::Running => "Running".to_string(),
            TaskStatus::Paused => "Paused".to_string(),
            TaskStatus::Completed => "Completed".to_string(),
            TaskStatus::Failed => "Failed".to_string(),
            TaskStatus::WaitingForApproval => "WaitingForApproval".to_string(),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        TaskStatus::from_str(s)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Task {
    pub id: i32,
    pub name: String,
    pub title: String,         // 任务标题
    pub task_type: String,     // 任务类型，如"publish", "backup", "restore"等
    pub app_name: String,      // 关联的应用名称
    pub status: TaskStatus,    // 任务状态
    pub progress: f32,         // 进度百分比 (0.0-100.0)
    pub total_items: i32,      // 总项目数（如总文件数、总chunk数等）
    pub completed_items: i32,  // 已完成项目数
    pub error_message: Option<String>, // 错误信息（如果有）
    pub data: Option<String>,  // 任务相关的JSON数据，可存储任何任务特定信息
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(name: String, title: String, task_type: String, app_name: String, data: Option<String>) -> Self {
        Task {
            id: 0,
            name,
            title,
            task_type,
            app_name,
            status: TaskStatus::Pending,
            progress: 0.0,
            total_items: 0,
            completed_items: 0,
            error_message: None,
            data,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
    
    pub fn update_progress(&mut self, completed_items: i32, total_items: i32) {
        self.completed_items = completed_items;
        self.total_items = total_items;
        if total_items > 0 {
            self.progress = (completed_items as f32 / total_items as f32) * 100.0;
        }
        self.updated_at = Utc::now();
    }
    
    pub fn set_error(&mut self, error_message: String) {
        self.error_message = Some(error_message);
        self.status = TaskStatus::Failed;
        self.updated_at = Utc::now();
    }
    
    pub fn update_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }
    
    pub fn update_data(&mut self, data: String) {
        self.data = Some(data);
        self.updated_at = Utc::now();
    }
}
