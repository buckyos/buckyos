这是一个综合了**状态快照（State Snapshot）**、**命名空间隔离（Namespaces）**、**权限控制（ACL）**以及**父子任务编排（Hierarchical Tasks）**的完整设计方案。

该文件包含了：

1. **数据模型**：定义了任务、状态、权限的核心结构。
2. **DTO (Data Transfer Objects)**：定义了 RPC 通信用的参数结构。
3. **客户端实现**：封装了 RPC 调用，提供了符合 Rust 惯用法的 SDK。

```rust
// task_mgr.rs
// BuckyOS TaskManager Service Design & Interface
//
// Design Philosophy:
// 1. Resource-Based: Tasks are state containers.
// 2. Atomic: Updates to status, progress, and data happen in snapshots.
// 3. Secure: Identity is enforced by Context; Access is controlled by Scopes.
// 4. Hierarchical: Supports parent/child relationships for complex job orchestration.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

// ============================================================================
// 1. Data Models (The "State")
// ============================================================================

/// 任务状态枚举
/// 使用 String 序列化，保证对前端/其他语言的兼容性
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,            // 已创建但未开始
    Running,            // 正在执行
    Paused,             // 已暂停（保留资源）
    Completed,          // 成功结束
    Failed,             // 失败结束
    Canceled,           // 被取消（类似 Failed，但是用户主动触发）
    WaitingForApproval, // 阻塞等待人工介入
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// 权限范围等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Serialize, Deserialize)]
pub enum TaskScope {
    Private, // 仅 Owner App 可见/操作
    User,    // 同 User 下的所有 App 可见/操作
    System,  // 全局可见 (需特殊权限创建)
}

/// 任务权限控制列表 (ACL)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPermissions {
    pub read: TaskScope,  // 谁可以 list/get 这个任务？
    pub write: TaskScope, // 谁可以 update/pause/cancel 这个任务？
}

impl Default for TaskPermissions {
    fn default() -> Self {
        Self {
            read: TaskScope::User,    // 默认：用户可以看到所有 App 的任务
            write: TaskScope::Private, // 默认：只有创建任务的 App 能控制它
        }
    }
}

/// 核心任务结构体 (The Resource)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    // --- 唯一标识 ---
    pub id: i64,

    // --- 身份与隔离 (由服务端基于 Context 注入，客户端不可伪造) ---
    pub user_id: String, // 任务归属的用户
    pub app_id: String,  // 任务归属的应用 (Owner)

    // --- 层级关系 ---
    pub parent_id: Option<i64>, // 父任务 ID
    pub root_id: Option<i64>,   // 根任务 ID (优化：方便快速查找整棵树)

    // --- 核心状态快照 ---
    pub name: String,
    pub task_type: String,      // 任务类型标识 (e.g., "download", "backup", "ai_inference")
    pub status: TaskStatus,
    pub progress: f32,          // 0.0 - 100.0
    pub message: Option<String>,// 当前状态描述 (e.g., "Copying file 5/100", "Disk Full")
    
    // --- 扩展数据 ---
    // 所有的业务特定数据 (total_bytes, speed, download_url, etc.) 都放在这里
    pub data: Value,

    // --- 权限 ---
    pub permissions: TaskPermissions,

    // --- 时间戳 ---
    pub created_at: u64,
    pub updated_at: u64,
}

// ============================================================================
// 2. Request/Response DTOs (The "Interface")
// ============================================================================

/// 创建任务的可选参数
#[derive(Debug, Default, Serialize)]
pub struct CreateTaskOptions {
    pub permissions: Option<TaskPermissions>,
    pub parent_id: Option<i64>,
    pub priority: Option<u8>, // 0-255, default 128
}

/// 任务过滤条件
#[derive(Debug, Default, Serialize)]
pub struct TaskFilter {
    pub app_id: Option<String>,      // 筛选特定 App
    pub task_type: Option<String>,   // 筛选特定类型
    pub status: Option<TaskStatus>,  // 筛选特定状态
    pub parent_id: Option<i64>,      // 筛选特定父任务的直接子任务
    pub root_id: Option<i64>,        // 筛选特定任务树
    // 注意：服务端会强制附加 user_id = current_user 过滤
}

/// 任务更新载荷 (Atomic Update)
/// 所有字段均为 Option，None 表示不修改
#[derive(Debug, Default, Serialize)]
pub struct TaskUpdatePayload {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>, // 建议实现为 JSON Merge Patch
}

// ============================================================================
// 3. Client Implementation
// ============================================================================

// 模拟 kRPC 客户端依赖
mod k_rpc {
    use serde_json::Value;
    // 这是一个 Mock，实际项目中引入真实的 kRPC crate
    pub struct KRPC; 
    impl KRPC {
        pub async fn call(&self, _method: &str, _params: Value) -> Result<Value, String> {
            Ok(Value::Null) // Mock return
        }
    }
}
use k_rpc::KRPC as kRPC;

/// TaskManager 客户端
pub struct TaskManagerClient {
    rpc: kRPC,
}

impl TaskManagerClient {
    pub fn new(rpc: kRPC) -> Self {
        Self { rpc }
    }

    /// 内部通用请求处理
    async fn request<T: for<'de> Deserialize<'de>>(&self, method: &str, params: Value) -> Result<T, String> {
        let resp = self.rpc.call(method, params).await?;
        // 假设服务端返回格式为 { "result": T } 或直接为 T
        serde_json::from_value(resp).map_err(|e| e.to_string())
    }

    // ------------------------------------------------------------------------
    // CRUD Operations
    // ------------------------------------------------------------------------

    /// 创建新任务
    /// 
    /// `name`: 人类可读名称
    /// `task_type`: 机器可读类型
    /// `data`: 初始扩展数据
    /// `opts`: 权限、父任务等高级选项
    pub async fn create_task(
        &self,
        name: &str,
        task_type: &str,
        data: Option<Value>,
        opts: Option<CreateTaskOptions>,
    ) -> Result<Task, String> {
        let opts = opts.unwrap_or_default();
        let params = json!({
            "name": name,
            "task_type": task_type,
            "data": data.unwrap_or(json!({})),
            "permissions": opts.permissions,
            "parent_id": opts.parent_id,
            "priority": opts.priority
        });

        // 服务端会自动从 Context 提取 user_id 和 app_id 并写入 Task
        self.request("create_task", params).await
    }

    /// 获取单个任务详情
    pub async fn get_task(&self, id: i64) -> Result<Task, String> {
        self.request("get_task", json!({ "id": id })).await
    }

    /// 列出任务 (支持过滤)
    pub async fn list_tasks(&self, filter: Option<TaskFilter>) -> Result<Vec<Task>, String> {
        let params = serde_json::to_value(filter.unwrap_or_default()).unwrap();
        self.request("list_tasks", params).await
    }

    // ------------------------------------------------------------------------
    // State Management (Atomic & Snapshot)
    // ------------------------------------------------------------------------

    /// 原子化更新任务快照
    /// 允许同时更新状态、进度、消息和数据，避免 UI 闪烁和状态不一致
    pub async fn update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data_patch: Option<Value>,
    ) -> Result<(), String> {
        let payload = TaskUpdatePayload {
            id,
            status,
            progress,
            message,
            data: data_patch,
        };
        let params = serde_json::to_value(payload).unwrap();
        self.request("update_task", params).await.map(|_: Value| ())
    }

    /// 快捷方法：报告进度
    pub async fn report_progress(&self, id: i64, progress: f32, msg: Option<&str>) -> Result<(), String> {
        self.update_task(id, None, Some(progress), msg.map(|s| s.to_string()), None).await
    }

    /// 快捷方法：标记成功
    pub async fn mark_completed(&self, id: i64, final_data: Option<Value>) -> Result<(), String> {
        self.update_task(id, Some(TaskStatus::Completed), Some(100.0), None, final_data).await
    }

    /// 快捷方法：标记失败
    pub async fn mark_failed(&self, id: i64, error_msg: &str) -> Result<(), String> {
        self.update_task(
            id, 
            Some(TaskStatus::Failed), 
            None, 
            Some(error_msg.to_string()), 
            None
        ).await
    }

    // ------------------------------------------------------------------------
    // Control Flow & Hierarchy
    // ------------------------------------------------------------------------

    /// 暂停任务
    /// 如果是父任务，服务端负责级联暂停所有子任务
    pub async fn pause_task(&self, id: i64) -> Result<(), String> {
        self.update_task(id, Some(TaskStatus::Paused), None, Some("Paused by user".into()), None).await
    }

    /// 恢复任务
    /// 如果是父任务，服务端负责级联恢复
    pub async fn resume_task(&self, id: i64) -> Result<(), String> {
        self.update_task(id, Some(TaskStatus::Running), None, Some("Resumed".into()), None).await
    }

    /// 取消任务
    /// `recursive`: 是否强制取消整个任务树。
    /// 如果 `false` 且该任务有子任务，可能会返回错误或只取消自身。
    pub async fn cancel_task(&self, id: i64, recursive: bool) -> Result<(), String> {
        let params = json!({
            "id": id,
            "status": TaskStatus::Canceled,
            "recursive": recursive
        });
        // 这里调用专门的 cancel 接口而不是 update，因为可能涉及复杂的资源清理逻辑
        self.request("cancel_task", params).await.map(|_: Value| ())
    }

    /// 获取子任务列表
    pub async fn get_subtasks(&self, parent_id: i64) -> Result<Vec<Task>, String> {
        let filter = TaskFilter {
            parent_id: Some(parent_id),
            ..Default::default()
        };
        self.list_tasks(Some(filter)).await
    }
}

```

### 设计说明 (Key Design Notes)

1. **原子更新 (`update_task`)**：
这是最重要的接口。它允许 App 在一行代码中完成状态流转。例如，从 `Running` 变为 `Failed` 时，同时写入错误信息 `Disk Full`，避免了 UI 看到“失败”但没有错误原因的瞬间状态。
2. **安全模型 (Security Model)**：
客户端接口中没有任何地方传递 `user_id`。
* **读权限**：`list_tasks` 时，服务端隐式添加 `WHERE user_id = ctx.user_id`，除非调用者有 Root 权限。
* **写权限**：更新任务时，服务端检查 `ctx.app_id == task.app_id` (Private Scope) 或 `task.permissions.write == User`。


3. **扩展数据 (`data: Value`)**：
不再通过结构体字段（如 `completed_items`）限制业务。
* 下载任务存：`{ "url": "...", "speed": "5MB/s" }`
* AI 任务存：`{ "model": "gpt4", "tokens_processed": 500 }`
UI 层根据 `task_type` 决定如何渲染 `data`。


4. **父子任务 (Hierarchy)**：
* **存储**：通过 `parent_id` 扁平存储，数据库友好。
* **操作**：`cancel_task` 提供了 `recursive` 标志，允许一键终止整个任务树。
* **查询**：提供了 `root_id` 字段。如果我想查整个“安装 Office”及其所有子步骤，只需 `filter.root_id = x`，无需递归 SQL 查询。