use serde_json::Value;

pub use buckyos_api::{Task, TaskPermissions, TaskScope, TaskStatus};

pub fn new_task(
    name: String,
    task_type: String,
    user_id: String,
    app_id: String,
    parent_id: Option<i64>,
    permissions: TaskPermissions,
    data: Value,
) -> Task {
    let now = chrono::Utc::now().timestamp() as u64;
    Task {
        id: 0,
        user_id,
        app_id,
        parent_id,
        root_id: String::new(),
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
