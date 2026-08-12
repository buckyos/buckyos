//! Agent Todo CLI — `todo` 与 `delegateTask` 工具实现。
//!
//! 设计意图见 `doc/opendan/Agent TodoList.md`：Todo 是 LLM Behavior 树的一个
//! 分支节点（fork-join 执行模型），工具面刻意保持极小：5 个 prompt-visible 工具
//! （`addTodo` / `currentTodo` / `listTodo` / `finishTodo` / `delegateTask`），
//! 通过 `todo` CLI 的子命令落地，加上独立的 `delegateTask` 入口。
//!
//! 路径解析（见文档 §13.3 / §13.4）：
//! - session 级 `todos.json` —— `<agent_rootfs>/sessions/<session_id>/todos.json`
//! - workspace 级 `tasks.json` —— `<workspace_dir>/.agent/tasks.json`
//!
//! `agent_rootfs` 在工具构造时注入；`session_id` 来自 `ToolCtx::session()`；
//! `workspace_dir` 取 `ToolCtx::shell_cwd()`（即 `exec_bash` 的 cwd），缺省时
//! 回退到 cwd 向上找最近一层带 `.agent/` 的目录。

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use fs2::FileExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::tool::CallingConventions;
use crate::{AgentToolError, CliInvocation, ToolCtx, TypedTool};

pub const TOOL_TODO: &str = "todo";
pub const TOOL_DELEGATE_TASK: &str = "delegateTask";

const MAX_SKILLS_PER_TODO: usize = 3;

// ---------------------------------------------------------------------------
// Data model — 对齐 `doc/opendan/Agent TodoList.md` §9
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Timeout,
    Blocked,
}

impl TodoStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TodoStatus::Completed | TodoStatus::Failed | TodoStatus::Timeout
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TodoRecord {
    pub todo_id: String,
    pub session_id: String,
    pub order_index: usize,
    pub status: TodoStatus,
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskEntry {
    pub task_id: String,
    pub purpose: String,
    pub origin_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_at: Option<String>,
    pub added_at: String,
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Todo / DelegateTask 工具共享的运行时配置。
///
/// `agent_rootfs` 对应文档 §13.3 中的 `$AGENT_ROOTFS`：当前 agent 在 host 上的
/// 可写根。`sessions/<session_id>/` 与 `workspaces/<workspace_id>/` 都挂在它下面。
#[derive(Clone, Debug)]
pub struct TodoToolConfig {
    pub agent_rootfs: PathBuf,
}

impl TodoToolConfig {
    pub fn new(agent_rootfs: impl Into<PathBuf>) -> Self {
        Self {
            agent_rootfs: agent_rootfs.into(),
        }
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.agent_rootfs.join("sessions").join(session_id)
    }

    fn todos_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("todos.json")
    }
}

// ---------------------------------------------------------------------------
// 委托后端：把生成 / 注册系统级 Task 的实际工作交给可插拔实现。
//
// 默认实现 [`StubTaskDelegator`] 直接 mint 一个本地 ID 作为占位，便于在没接入
// task_management 后端时也能跑通 Todo 工具的写路径。生产路径里替换为真实的
// TaskMgr 客户端即可。
// ---------------------------------------------------------------------------

#[async_trait]
pub trait TaskDelegator: Send + Sync {
    /// 发起一个系统级 Task。返回 `(taskID, status)`。
    async fn delegate(
        &self,
        ctx: &ToolCtx<'_>,
        task: &str,
        to: Option<&str>,
        context: Option<&str>,
    ) -> Result<(String, String), AgentToolError>;
}

#[derive(Default, Clone, Debug)]
pub struct StubTaskDelegator;

#[async_trait]
impl TaskDelegator for StubTaskDelegator {
    async fn delegate(
        &self,
        _ctx: &ToolCtx<'_>,
        task: &str,
        _to: Option<&str>,
        _context: Option<&str>,
    ) -> Result<(String, String), AgentToolError> {
        // 时间戳 + 任务摘要做一次 blake3，截前 12 hex 当 ID。
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut hasher = blake3::Hasher::new();
        hasher.update(&now.to_be_bytes());
        hasher.update(task.as_bytes());
        let id = format!("tsk_{}", &hasher.finalize().to_hex().as_str()[..12]);
        Ok((id, "pending".to_string()))
    }
}

// ---------------------------------------------------------------------------
// 路径与文件 IO 助手
// ---------------------------------------------------------------------------

fn resolve_workspace_dir(ctx: &ToolCtx<'_>) -> Result<PathBuf, AgentToolError> {
    if let Some(cwd) = ctx.shell_cwd() {
        return Ok(cwd.to_path_buf());
    }
    // 兜底：从当前进程 cwd 向上找带 `.agent/` 的目录；找不到就用 cwd 本身。
    let cwd = std::env::current_dir()
        .map_err(|err| AgentToolError::ExecFailed(format!("get cwd failed: {err}")))?;
    let mut cursor: &Path = cwd.as_path();
    loop {
        if cursor.join(".agent").is_dir() {
            return Ok(cursor.to_path_buf());
        }
        match cursor.parent() {
            Some(parent) => cursor = parent,
            None => return Ok(cwd.clone()),
        }
    }
}

fn tasks_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(".agent").join("tasks.json")
}

/// 以排他锁打开 JSON meta 文件，读现有内容后允许调用方就地改写并落盘。
///
/// 文件不存在时按空 JSON 数组初始化。锁基于 `fs2::FileExt::lock_exclusive`，
/// 与文档 §12-8（meta 并发写）保持一致：先用最简单的文件锁兜住同机多 session
/// 的竞争，DAG / 日志合并属于后续优化。
fn read_modify_write<T, F, R>(path: &Path, mutate: F) -> Result<R, AgentToolError>
where
    T: serde::de::DeserializeOwned + Serialize + Default,
    F: FnOnce(&mut T) -> Result<R, AgentToolError>,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AgentToolError::ExecFailed(format!("create dir {} failed: {err}", parent.display()))
        })?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("open {} failed: {err}", path.display()))
        })?;
    FileExt::lock_exclusive(&file).map_err(|err| {
        AgentToolError::ExecFailed(format!("lock {} failed: {err}", path.display()))
    })?;

    let mut buf = String::new();
    file.read_to_string(&mut buf).map_err(|err| {
        AgentToolError::ExecFailed(format!("read {} failed: {err}", path.display()))
    })?;
    let mut data: T = if buf.trim().is_empty() {
        T::default()
    } else {
        serde_json::from_str(&buf).map_err(|err| {
            AgentToolError::ExecFailed(format!("parse {} failed: {err}", path.display()))
        })?
    };

    let outcome = mutate(&mut data)?;

    let serialized = serde_json::to_string_pretty(&data).map_err(|err| {
        AgentToolError::ExecFailed(format!("serialize {} failed: {err}", path.display()))
    })?;
    file.set_len(0).map_err(|err| {
        AgentToolError::ExecFailed(format!("truncate {} failed: {err}", path.display()))
    })?;
    file.seek(SeekFrom::Start(0)).map_err(|err| {
        AgentToolError::ExecFailed(format!("seek {} failed: {err}", path.display()))
    })?;
    file.write_all(serialized.as_bytes()).map_err(|err| {
        AgentToolError::ExecFailed(format!("write {} failed: {err}", path.display()))
    })?;
    file.flush().ok();
    FileExt::unlock(&file).ok();
    Ok(outcome)
}

pub fn read_todo_records(path: &Path) -> Result<Vec<TodoRecord>, AgentToolError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = File::open(path).map_err(|err| {
        AgentToolError::ExecFailed(format!("open {} failed: {err}", path.display()))
    })?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).map_err(|err| {
        AgentToolError::ExecFailed(format!("read {} failed: {err}", path.display()))
    })?;
    if buf.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&buf).map_err(|err| {
        AgentToolError::ExecFailed(format!("parse {} failed: {err}", path.display()))
    })
}

fn read_todos(path: &Path) -> Result<Vec<TodoRecord>, AgentToolError> {
    read_todo_records(path)
}

fn read_tasks(path: &Path) -> Result<Vec<TaskEntry>, AgentToolError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = File::open(path).map_err(|err| {
        AgentToolError::ExecFailed(format!("open {} failed: {err}", path.display()))
    })?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).map_err(|err| {
        AgentToolError::ExecFailed(format!("read {} failed: {err}", path.display()))
    })?;
    if buf.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&buf).map_err(|err| {
        AgentToolError::ExecFailed(format!("parse {} failed: {err}", path.display()))
    })
}

fn next_todo_id(existing: &[TodoRecord]) -> String {
    // ID 形如 T01；超过 99 自动多一位，保持递增不变。文档 §12-1 把固定长度
    // 列为 TBD，这里取“按需增长，最少两位”作为可工作的默认。
    let next = existing.len() + 1;
    format!("T{:02}", next)
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// TodoTool — todo {add|current|list|done|failed|finish|show}
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TodoArgs {
    Add {
        title: String,
        content: String,
        #[serde(default)]
        skills: Vec<String>,
        #[serde(default)]
        context: Option<String>,
    },
    Current,
    List,
    Done {
        summary: String,
        #[serde(default)]
        report: Option<String>,
        #[serde(default)]
        report_file: Option<String>,
        #[serde(default)]
        todo_id: Option<String>,
    },
    Failed {
        summary: String,
        #[serde(default)]
        report: Option<String>,
        #[serde(default)]
        report_file: Option<String>,
        #[serde(default)]
        todo_id: Option<String>,
    },
    Finish {
        status: TodoStatus,
        summary: String,
        #[serde(default)]
        report: Option<String>,
        #[serde(default)]
        report_file: Option<String>,
        #[serde(default)]
        todo_id: Option<String>,
    },
    Show {
        #[serde(default)]
        todo_id: Option<String>,
    },
    /// Replan 专用：删除所有 `pending` Todo（未执行过、无外部副作用）。
    /// 不动 `running/blocked` 及任何终态记录，保留已动过的执行审计。
    Clean,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TodoOutput {
    Add {
        todo_id: String,
        status: TodoStatus,
        order_index: usize,
        is_current: bool,
    },
    Current {
        todo: Option<TodoView>,
    },
    List {
        todos: Vec<TodoListItem>,
        tasks: Vec<TaskListItem>,
    },
    Done {
        todo_id: String,
        status: TodoStatus,
    },
    Failed {
        todo_id: String,
        status: TodoStatus,
    },
    Finish {
        todo_id: String,
        status: TodoStatus,
    },
    Show {
        todo: Option<TodoView>,
    },
    Clean {
        removed: Vec<String>,
        kept: usize,
    },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct TodoView {
    pub todo_id: String,
    pub status: TodoStatus,
    pub title: String,
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    pub order_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
}

impl From<&TodoRecord> for TodoView {
    fn from(r: &TodoRecord) -> Self {
        Self {
            todo_id: r.todo_id.clone(),
            status: r.status,
            title: r.title.clone(),
            content: r.content.clone(),
            skills: r.skills.clone(),
            order_index: r.order_index,
            creation_context: r.creation_context.clone(),
            report: r.report.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct TodoListItem {
    pub id: String,
    pub r#type: String, // "todo"
    pub status: TodoStatus,
    pub order_index: usize,
    pub is_current: bool,
    pub title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct TaskListItem {
    pub id: String,
    pub r#type: String, // "task"
    pub status: String,
    pub purpose: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct TodoTool {
    cfg: TodoToolConfig,
}

impl TodoTool {
    pub fn new(cfg: TodoToolConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl TypedTool for TodoTool {
    type Args = TodoArgs;
    type Output = TodoOutput;

    fn name(&self) -> &str {
        TOOL_TODO
    }

    fn description(&self) -> &str {
        "Session 级 Todo 工具：plan 写入下一步执行分支、DO 拉取 current todo、\
         分支结束写 summary/report 回流到主干。"
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    fn usage(&self) -> Option<String> {
        Some(
            "todo add \"<title>\" --content <text> [--skill <name>]... [--context <text>]\n\
             todo current\n\
             todo list\n\
             todo done \"<summary>\" [--report <text> | --report-file <path>]\n\
             todo failed \"<summary>\" [--report <text> | --report-file <path>]\n\
             todo finish --status <completed|failed|timeout|blocked> \"<summary>\" \
                [--report <text> | --report-file <path>] [--id <todoID>]\n\
             todo show [<todoID>]\n\
             todo clean"
                .to_string(),
        )
    }

    fn parse_bash_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        parse_todo_cli_tokens(tokens)
    }

    fn parse_cli_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<CliInvocation, AgentToolError> {
        let args = parse_todo_cli_tokens(tokens)?;
        Ok(CliInvocation::Json {
            args,
            content_input: None,
        })
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let line = match args {
            TodoArgs::Add { .. } => "todo add",
            TodoArgs::Current => "todo current",
            TodoArgs::List => "todo list",
            TodoArgs::Done { .. } => "todo done",
            TodoArgs::Failed { .. } => "todo failed",
            TodoArgs::Finish { .. } => "todo finish",
            TodoArgs::Show { .. } => "todo show",
            TodoArgs::Clean => "todo clean",
        };
        Some(line.to_string())
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        match output {
            TodoOutput::Add {
                todo_id,
                is_current,
                ..
            } => format!(
                "added {todo_id}{}",
                if *is_current { " (current)" } else { "" }
            ),
            TodoOutput::Current { todo: Some(t) } => {
                format!("current {} {:?}", t.todo_id, t.status)
            }
            TodoOutput::Current { todo: None } => "no current todo".to_string(),
            TodoOutput::List { todos, tasks } => {
                format!("{} todos / {} delegated tasks", todos.len(), tasks.len())
            }
            TodoOutput::Done { todo_id, status }
            | TodoOutput::Failed { todo_id, status }
            | TodoOutput::Finish { todo_id, status } => format!("{todo_id} -> {:?}", status),
            TodoOutput::Show { todo: Some(t) } => format!("show {} {:?}", t.todo_id, t.status),
            TodoOutput::Show { todo: None } => "no such todo".to_string(),
            TodoOutput::Clean { removed, kept } => {
                format!("cleaned {} pending, kept {kept}", removed.len())
            }
        }
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let session_id = ctx.session().session_id.trim().to_string();
        if session_id.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id is empty — runtime must inject WORK_SESSION_ID".to_string(),
            ));
        }
        let todos_path = self.cfg.todos_path(&session_id);

        match args {
            TodoArgs::Add {
                title,
                content,
                skills,
                context,
            } => execute_add(&todos_path, &session_id, title, content, skills, context).await,
            TodoArgs::Current => execute_current(&todos_path),
            TodoArgs::List => execute_list(ctx, &todos_path),
            TodoArgs::Done {
                summary,
                report,
                report_file,
                todo_id,
            } => {
                let report = resolve_report(ctx, report, report_file)?;
                execute_finish(
                    &todos_path,
                    todo_id.as_deref(),
                    TodoStatus::Completed,
                    summary,
                    report,
                    /*is_done_alias=*/ true,
                )
                .await
            }
            TodoArgs::Failed {
                summary,
                report,
                report_file,
                todo_id,
            } => {
                let report = resolve_report(ctx, report, report_file)?;
                execute_failed(&todos_path, todo_id.as_deref(), summary, report).await
            }
            TodoArgs::Finish {
                status,
                summary,
                report,
                report_file,
                todo_id,
            } => {
                let report = resolve_report(ctx, report, report_file)?;
                execute_finish(
                    &todos_path,
                    todo_id.as_deref(),
                    status,
                    summary,
                    report,
                    /*is_done_alias=*/ false,
                )
                .await
            }
            TodoArgs::Show { todo_id } => execute_show(&todos_path, todo_id.as_deref()),
            TodoArgs::Clean => execute_clean(&todos_path),
        }
    }
}

async fn execute_failed(
    todos_path: &Path,
    todo_id: Option<&str>,
    summary: String,
    report: Option<String>,
) -> Result<TodoOutput, AgentToolError> {
    match execute_finish(
        todos_path,
        todo_id,
        TodoStatus::Failed,
        summary,
        report,
        /*is_done_alias=*/ false,
    )
    .await?
    {
        TodoOutput::Finish { todo_id, status } => Ok(TodoOutput::Failed { todo_id, status }),
        other => Ok(other),
    }
}

async fn execute_add(
    todos_path: &Path,
    session_id: &str,
    title: String,
    content: String,
    skills: Vec<String>,
    context: Option<String>,
) -> Result<TodoOutput, AgentToolError> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "`title` must be non-empty".to_string(),
        ));
    }
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "`content` must be a non-empty natural-language description".to_string(),
        ));
    }
    if skills.len() > MAX_SKILLS_PER_TODO {
        return Err(AgentToolError::InvalidArgs(format!(
            "at most {MAX_SKILLS_PER_TODO} skills per todo (got {})",
            skills.len()
        )));
    }

    let now = now_iso();
    read_modify_write::<Vec<TodoRecord>, _, _>(todos_path, |list| {
        let todo_id = next_todo_id(list);
        let order_index = list.len();
        let record = TodoRecord {
            todo_id: todo_id.clone(),
            session_id: session_id.to_string(),
            order_index,
            status: TodoStatus::Pending,
            title: title.clone(),
            content: content.clone(),
            skills,
            creation_context: context,
            report: None,
            workspace: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        list.push(record);

        let is_current = list
            .iter()
            .find(|r| !r.status.is_terminal())
            .map(|r| r.todo_id == todo_id)
            .unwrap_or(false);
        Ok(TodoOutput::Add {
            todo_id,
            status: TodoStatus::Pending,
            order_index,
            is_current,
        })
    })
}

fn execute_current(todos_path: &Path) -> Result<TodoOutput, AgentToolError> {
    let todos = read_todos(todos_path)?;
    let view = todos
        .iter()
        .find(|r| !r.status.is_terminal())
        .map(TodoView::from);
    Ok(TodoOutput::Current { todo: view })
}

fn execute_list(ctx: &ToolCtx<'_>, todos_path: &Path) -> Result<TodoOutput, AgentToolError> {
    let todos = read_todos(todos_path)?;
    let current_id = todos
        .iter()
        .find(|r| !r.status.is_terminal())
        .map(|r| r.todo_id.clone());
    let todo_items: Vec<TodoListItem> = todos
        .iter()
        .map(|r| TodoListItem {
            id: r.todo_id.clone(),
            r#type: "todo".to_string(),
            status: r.status,
            order_index: r.order_index,
            is_current: Some(&r.todo_id) == current_id.as_ref(),
            title: first_line(&r.title, 80),
            skills: r.skills.clone(),
            updated_at: r.updated_at.clone(),
        })
        .collect();

    // Workspace 级 tasks.json：缺 workspace 时不报错，列空即可（脱机 bash 检查
    // 仍然能直接读那个文件）。
    let tasks = match resolve_workspace_dir(ctx) {
        Ok(workspace) => read_tasks(&tasks_path(&workspace)).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let task_items: Vec<TaskListItem> = tasks
        .iter()
        .map(|t| TaskListItem {
            id: t.task_id.clone(),
            r#type: "task".to_string(),
            status: t
                .cached_status
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            purpose: t.purpose.clone(),
            updated_at: t.cached_at.clone().unwrap_or_else(|| t.added_at.clone()),
        })
        .collect();

    Ok(TodoOutput::List {
        todos: todo_items,
        tasks: task_items,
    })
}

async fn execute_finish(
    todos_path: &Path,
    todo_id: Option<&str>,
    status: TodoStatus,
    summary: String,
    report: Option<String>,
    is_done_alias: bool,
) -> Result<TodoOutput, AgentToolError> {
    if is_done_alias && status != TodoStatus::Completed {
        // `todo done` 是 `finish --status completed` 的语义糖；非 completed 必须
        // 走 `todo finish` 以保留显式信号。
        return Err(AgentToolError::InvalidArgs(
            "`todo done` only sets status=completed; use `todo finish --status ...`".to_string(),
        ));
    }
    let summary = summary.trim().to_string();
    if summary.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "summary must be non-empty".to_string(),
        ));
    }
    let now = now_iso();
    let outcome = read_modify_write::<Vec<TodoRecord>, _, _>(todos_path, |list| {
        let idx = if let Some(id) = todo_id {
            list.iter()
                .position(|r| r.todo_id == id)
                .ok_or_else(|| AgentToolError::NotFound(format!("todo `{id}` not found")))?
        } else {
            list.iter()
                .position(|r| !r.status.is_terminal())
                .ok_or_else(|| {
                    AgentToolError::InvalidArgs("no current todo to finish".to_string())
                })?
        };
        let rec = &mut list[idx];
        rec.status = status;
        rec.report = report.or(Some(summary));
        rec.updated_at = now.clone();
        Ok((rec.todo_id.clone(), rec.status))
    })?;
    let (todo_id, status) = outcome;
    if is_done_alias {
        Ok(TodoOutput::Done { todo_id, status })
    } else {
        Ok(TodoOutput::Finish { todo_id, status })
    }
}

fn execute_show(todos_path: &Path, todo_id: Option<&str>) -> Result<TodoOutput, AgentToolError> {
    let todos = read_todos(todos_path)?;
    let view = match todo_id {
        Some(id) => todos.iter().find(|r| r.todo_id == id).map(TodoView::from),
        None => todos
            .iter()
            .find(|r| !r.status.is_terminal())
            .map(TodoView::from),
    };
    Ok(TodoOutput::Show { todo: view })
}

fn execute_clean(todos_path: &Path) -> Result<TodoOutput, AgentToolError> {
    // 只清 `pending`：未执行过，无外部副作用，replan 时配合后续 `addTodo` 重建
    // baseline。`running/blocked` 已经动过、`completed/failed/timeout` 是审计
    // 终态，按 §8.2 "防掩盖历史" 原则都保留。orderIndex 在清理后顺序压紧到
    // 0..N-1，保证 listTodo / currentTodo 的"按创建顺序"语义在 replan 之后仍
    // 然成立。
    read_modify_write::<Vec<TodoRecord>, _, _>(todos_path, |list| {
        let mut removed = Vec::new();
        let mut kept = Vec::with_capacity(list.len());
        for rec in std::mem::take(list).into_iter() {
            if rec.status == TodoStatus::Pending {
                removed.push(rec.todo_id);
            } else {
                kept.push(rec);
            }
        }
        for (idx, rec) in kept.iter_mut().enumerate() {
            rec.order_index = idx;
        }
        let kept_count = kept.len();
        *list = kept;
        Ok(TodoOutput::Clean {
            removed,
            kept: kept_count,
        })
    })
}

fn resolve_report(
    ctx: &ToolCtx<'_>,
    report: Option<String>,
    report_file: Option<String>,
) -> Result<Option<String>, AgentToolError> {
    match (report, report_file) {
        (Some(_), Some(_)) => Err(AgentToolError::InvalidArgs(
            "use exactly one of --report / --report-file".to_string(),
        )),
        (Some(text), None) => Ok(Some(text)),
        (None, Some(path)) => {
            let p = PathBuf::from(&path);
            let resolved = if p.is_absolute() {
                p
            } else if let Some(cwd) = ctx.shell_cwd() {
                cwd.join(p)
            } else {
                p
            };
            let body = std::fs::read_to_string(&resolved).map_err(|err| {
                AgentToolError::InvalidArgs(format!(
                    "read --report-file {} failed: {err}",
                    resolved.display()
                ))
            })?;
            Ok(Some(body))
        }
        (None, None) => Ok(None),
    }
}

fn first_line(s: &str, max_chars: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    if first.chars().count() <= max_chars {
        first.to_string()
    } else {
        let truncated: String = first.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

// ---------------------------------------------------------------------------
// CLI 解析 —— 子命令 + 位置参 + `--flag value` / `--flag=value` / 可重复 flag
// ---------------------------------------------------------------------------

fn parse_todo_cli_tokens(tokens: &[String]) -> Result<Json, AgentToolError> {
    let mut iter = tokens.iter();
    let sub = iter.next().map(String::as_str).ok_or_else(|| {
        AgentToolError::InvalidArgs(
            "todo requires a subcommand (add|current|list|done|failed|finish|show|clean)"
                .to_string(),
        )
    })?;
    let rest: Vec<String> = iter.cloned().collect();

    match sub {
        "add" => parse_add(&rest),
        "current" => {
            ensure_no_extra(&rest, "todo current")?;
            Ok(serde_json::json!({"op": "current"}))
        }
        "list" => {
            ensure_no_extra(&rest, "todo list")?;
            Ok(serde_json::json!({"op": "list"}))
        }
        "done" => parse_done(&rest),
        "failed" => parse_failed(&rest),
        "finish" => parse_finish(&rest),
        "show" => parse_show(&rest),
        "clean" => {
            ensure_no_extra(&rest, "todo clean")?;
            Ok(serde_json::json!({"op": "clean"}))
        }
        other => Err(AgentToolError::InvalidArgs(format!(
            "unknown todo subcommand `{other}` (expected add|current|list|done|failed|finish|show|clean)"
        ))),
    }
}

fn ensure_no_extra(rest: &[String], cmd: &str) -> Result<(), AgentToolError> {
    if !rest.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "{cmd} takes no arguments (got {:?})",
            rest
        )));
    }
    Ok(())
}

#[derive(Default)]
struct ParsedFlags {
    positionals: Vec<String>,
    single: std::collections::HashMap<String, String>,
    repeated: std::collections::HashMap<String, Vec<String>>,
}

fn parse_flags(tokens: &[String], repeated_flags: &[&str]) -> Result<ParsedFlags, AgentToolError> {
    let mut out = ParsedFlags::default();
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        if let Some(rest) = tok.strip_prefix("--") {
            // 支持 --flag=value 与 --flag value 两种形态。
            let (name, value) = if let Some(eq) = rest.find('=') {
                (rest[..eq].to_string(), Some(rest[eq + 1..].to_string()))
            } else {
                (rest.to_string(), None)
            };
            let value = match value {
                Some(v) => v,
                None => {
                    i += 1;
                    tokens.get(i).cloned().ok_or_else(|| {
                        AgentToolError::InvalidArgs(format!("--{name} requires a value"))
                    })?
                }
            };
            if repeated_flags.contains(&name.as_str()) {
                out.repeated.entry(name).or_default().push(value);
            } else if out.single.insert(name.clone(), value).is_some() {
                return Err(AgentToolError::InvalidArgs(format!(
                    "--{name} specified more than once"
                )));
            }
        } else {
            out.positionals.push(tok.clone());
        }
        i += 1;
    }
    Ok(out)
}

fn parse_add(tokens: &[String]) -> Result<Json, AgentToolError> {
    let flags = parse_flags(tokens, &["skill"])?;
    if flags.positionals.len() != 1 {
        return Err(AgentToolError::InvalidArgs(
            "todo add takes exactly one positional <title>".to_string(),
        ));
    }
    let title = flags.positionals.into_iter().next().unwrap();
    let content = flags.single.get("content").cloned().ok_or_else(|| {
        AgentToolError::InvalidArgs("todo add requires --content <text>".to_string())
    })?;
    let skills = flags.repeated.get("skill").cloned().unwrap_or_default();
    let context = flags.single.get("context").cloned();
    let unknown: Vec<&String> = flags
        .single
        .keys()
        .filter(|k| !matches!(k.as_str(), "content" | "context"))
        .collect();
    if !unknown.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "todo add: unknown flag(s) {:?}",
            unknown
        )));
    }
    Ok(serde_json::json!({
        "op": "add",
        "title": title,
        "content": content,
        "skills": skills,
        "context": context,
    }))
}

fn parse_done(tokens: &[String]) -> Result<Json, AgentToolError> {
    parse_terminal_alias(tokens, "done", "todo done")
}

fn parse_failed(tokens: &[String]) -> Result<Json, AgentToolError> {
    parse_terminal_alias(tokens, "failed", "todo failed")
}

fn parse_terminal_alias(tokens: &[String], op: &str, cmd: &str) -> Result<Json, AgentToolError> {
    let flags = parse_flags(tokens, &[])?;
    if flags.positionals.len() != 1 {
        return Err(AgentToolError::InvalidArgs(format!(
            "{cmd} takes exactly one positional <summary>"
        )));
    }
    let summary = flags.positionals.into_iter().next().unwrap();
    let report = flags.single.get("report").cloned();
    let report_file = flags.single.get("report-file").cloned();
    let unknown: Vec<&String> = flags
        .single
        .keys()
        .filter(|k| !matches!(k.as_str(), "report" | "report-file"))
        .collect();
    if !unknown.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "{cmd}: unknown flag(s) {unknown:?}"
        )));
    }
    Ok(serde_json::json!({
        "op": op,
        "summary": summary,
        "report": report,
        "report_file": report_file,
    }))
}

fn parse_finish(tokens: &[String]) -> Result<Json, AgentToolError> {
    let flags = parse_flags(tokens, &[])?;
    if flags.positionals.len() != 1 {
        return Err(AgentToolError::InvalidArgs(
            "todo finish takes exactly one positional <summary>".to_string(),
        ));
    }
    let summary = flags.positionals.into_iter().next().unwrap();
    let status = flags
        .single
        .get("status")
        .cloned()
        .ok_or_else(|| AgentToolError::InvalidArgs("--status is required".to_string()))?;
    let report = flags.single.get("report").cloned();
    let report_file = flags.single.get("report-file").cloned();
    let todo_id = flags.single.get("id").cloned();
    let unknown: Vec<&String> = flags
        .single
        .keys()
        .filter(|k| !matches!(k.as_str(), "status" | "report" | "report-file" | "id"))
        .collect();
    if !unknown.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "todo finish: unknown flag(s) {:?}",
            unknown
        )));
    }
    Ok(serde_json::json!({
        "op": "finish",
        "status": status,
        "summary": summary,
        "report": report,
        "report_file": report_file,
        "todo_id": todo_id,
    }))
}

fn parse_show(tokens: &[String]) -> Result<Json, AgentToolError> {
    let flags = parse_flags(tokens, &[])?;
    if flags.positionals.len() > 1 {
        return Err(AgentToolError::InvalidArgs(
            "todo show takes at most one positional <todoID>".to_string(),
        ));
    }
    if !flags.single.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "todo show: unknown flag(s) {:?}",
            flags.single.keys().collect::<Vec<_>>()
        )));
    }
    let todo_id = flags.positionals.into_iter().next();
    Ok(serde_json::json!({
        "op": "show",
        "todo_id": todo_id,
    }))
}

// ---------------------------------------------------------------------------
// DelegateTaskTool — delegateTask "<task>" [--to <target>] [--context <text>]
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct DelegateTaskArgs {
    pub task: String,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DelegateTaskOutput {
    pub task_id: String,
    pub status: String,
    pub purpose: String,
}

pub struct DelegateTaskTool {
    delegator: std::sync::Arc<dyn TaskDelegator>,
}

impl DelegateTaskTool {
    pub fn new(delegator: std::sync::Arc<dyn TaskDelegator>) -> Self {
        Self { delegator }
    }

    pub fn with_stub() -> Self {
        Self::new(std::sync::Arc::new(StubTaskDelegator))
    }
}

#[async_trait]
impl TypedTool for DelegateTaskTool {
    type Args = DelegateTaskArgs;
    type Output = DelegateTaskOutput;

    fn name(&self) -> &str {
        TOOL_DELEGATE_TASK
    }

    fn description(&self) -> &str {
        "委托一个系统级 Task（task_management 创建 / 路由），并把 taskID 登记到\
         当前 workspace 的 .agent/tasks.json。"
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    fn usage(&self) -> Option<String> {
        Some("delegateTask \"<task>\" [--to <target>] [--context <text>]".to_string())
    }

    fn parse_bash_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        parse_delegate_cli_tokens(tokens)
    }

    fn parse_cli_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<CliInvocation, AgentToolError> {
        let args = parse_delegate_cli_tokens(tokens)?;
        Ok(CliInvocation::Json {
            args,
            content_input: None,
        })
    }

    fn build_cmd_line(&self, _args: &Self::Args) -> Option<String> {
        Some("delegateTask".to_string())
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!("delegated {} ({})", output.task_id, output.status)
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let task = args.task.trim().to_string();
        if task.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "`task` must be a non-empty natural-language description".to_string(),
            ));
        }
        let session_id = ctx.session().session_id.clone();
        let purpose = args
            .context
            .clone()
            .unwrap_or_else(|| first_line(&task, 120));

        let (task_id, status) = self
            .delegator
            .delegate(ctx, &task, args.to.as_deref(), args.context.as_deref())
            .await?;

        // workspace 级登记。缺 workspace 时直接报错——delegated task 必须有
        // 可见的落地点，不能静默丢失（见文档 §7.0）。
        let workspace = resolve_workspace_dir(ctx)?;
        let path = tasks_path(&workspace);
        let now = now_iso();
        let entry = TaskEntry {
            task_id: task_id.clone(),
            purpose: purpose.clone(),
            origin_session_id: session_id,
            label: args.to.clone(),
            cached_status: Some(status.clone()),
            cached_at: Some(now.clone()),
            added_at: now,
        };
        read_modify_write::<Vec<TaskEntry>, _, _>(&path, |list| {
            list.push(entry);
            Ok(())
        })?;

        Ok(DelegateTaskOutput {
            task_id,
            status,
            purpose,
        })
    }
}

fn parse_delegate_cli_tokens(tokens: &[String]) -> Result<Json, AgentToolError> {
    let flags = parse_flags(tokens, &[])?;
    if flags.positionals.len() != 1 {
        return Err(AgentToolError::InvalidArgs(
            "delegateTask takes exactly one positional <task> description".to_string(),
        ));
    }
    let task = flags.positionals.into_iter().next().unwrap();
    let to = flags.single.get("to").cloned();
    let context = flags.single.get("context").cloned();
    let unknown: Vec<&String> = flags
        .single
        .keys()
        .filter(|k| !matches!(k.as_str(), "to" | "context"))
        .collect();
    if !unknown.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "delegateTask: unknown flag(s) {:?}",
            unknown
        )));
    }
    Ok(serde_json::json!({
        "task": task,
        "to": to,
        "context": context,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::NullToolHost;
    use crate::SessionRuntimeContext;
    use tempfile::tempdir;

    fn session(id: &str) -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "plan".into(),
            step_idx: 0,
            wakeup_id: "w".into(),
            session_id: id.into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        }
    }

    fn make_tool() -> (TodoTool, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let tool = TodoTool::new(TodoToolConfig::new(dir.path()));
        (tool, dir)
    }

    fn run_with_cwd<F, Fut, R>(
        tool: &TodoTool,
        sess: &SessionRuntimeContext,
        host: &NullToolHost,
        cwd: Option<&Path>,
        f: F,
    ) -> R
    where
        F: FnOnce(ToolCtx<'_>) -> Fut,
        Fut: std::future::Future<Output = R>,
    {
        let _ = tool; // silence unused
        let ctx = ToolCtx::new(sess, host).with_shell_cwd(cwd);
        tokio::runtime::Runtime::new().unwrap().block_on(f(ctx))
    }

    #[tokio::test]
    async fn add_then_current_then_done_cycles_through_list() {
        let (tool, _dir) = make_tool();
        let sess = session("sess-001");
        let host = NullToolHost;
        let ws = tempdir().unwrap();

        // add A
        let ctx = ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path()));
        let out = tool
            .execute(
                &ctx,
                TodoArgs::Add {
                    title: "write docs".into(),
                    content: "write docs details".into(),
                    skills: vec!["docs".into()],
                    context: None,
                },
            )
            .await
            .unwrap();
        match out {
            TodoOutput::Add {
                todo_id,
                is_current,
                order_index,
                ..
            } => {
                assert_eq!(todo_id, "T01");
                assert!(is_current);
                assert_eq!(order_index, 0);
            }
            _ => panic!("expected Add"),
        }

        // add B
        let _ = tool
            .execute(
                &ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path())),
                TodoArgs::Add {
                    title: "ship feature".into(),
                    content: "ship feature details".into(),
                    skills: vec![],
                    context: None,
                },
            )
            .await
            .unwrap();

        // current = T01
        let cur = tool
            .execute(
                &ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path())),
                TodoArgs::Current,
            )
            .await
            .unwrap();
        match cur {
            TodoOutput::Current { todo: Some(t) } => assert_eq!(t.todo_id, "T01"),
            _ => panic!("expected current T01"),
        }

        // done current
        let done = tool
            .execute(
                &ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path())),
                TodoArgs::Done {
                    summary: "docs landed".into(),
                    report: None,
                    report_file: None,
                    todo_id: None,
                },
            )
            .await
            .unwrap();
        match done {
            TodoOutput::Done { todo_id, status } => {
                assert_eq!(todo_id, "T01");
                assert_eq!(status, TodoStatus::Completed);
            }
            _ => panic!("expected Done"),
        }

        // current advances to T02
        let cur = tool
            .execute(
                &ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path())),
                TodoArgs::Current,
            )
            .await
            .unwrap();
        match cur {
            TodoOutput::Current { todo: Some(t) } => assert_eq!(t.todo_id, "T02"),
            _ => panic!("expected current T02"),
        }
    }

    #[tokio::test]
    async fn finish_with_failed_keeps_advancing() {
        let (tool, _dir) = make_tool();
        let sess = session("sess-x");
        let host = NullToolHost;
        let ws = tempdir().unwrap();
        let make_ctx = || ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path()));

        tool.execute(
            &make_ctx(),
            TodoArgs::Add {
                title: "a".into(),
                content: "a details".into(),
                skills: vec![],
                context: None,
            },
        )
        .await
        .unwrap();
        tool.execute(
            &make_ctx(),
            TodoArgs::Finish {
                status: TodoStatus::Failed,
                summary: "blocked by upstream".into(),
                report: None,
                report_file: None,
                todo_id: None,
            },
        )
        .await
        .unwrap();
        let cur = tool.execute(&make_ctx(), TodoArgs::Current).await.unwrap();
        // failed is terminal — no current todo.
        match cur {
            TodoOutput::Current { todo } => assert!(todo.is_none()),
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn failed_alias_marks_current_failed() {
        let (tool, _dir) = make_tool();
        let sess = session("sess-failed");
        let host = NullToolHost;
        let ws = tempdir().unwrap();
        let make_ctx = || ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path()));

        tool.execute(
            &make_ctx(),
            TodoArgs::Add {
                title: "investigate failure".into(),
                content: "investigate failure details".into(),
                skills: vec![],
                context: None,
            },
        )
        .await
        .unwrap();

        let failed = tool
            .execute(
                &make_ctx(),
                TodoArgs::Failed {
                    summary: "upstream is unavailable".into(),
                    report: None,
                    report_file: None,
                    todo_id: None,
                },
            )
            .await
            .unwrap();
        match failed {
            TodoOutput::Failed { todo_id, status } => {
                assert_eq!(todo_id, "T01");
                assert_eq!(status, TodoStatus::Failed);
            }
            _ => panic!("expected Failed"),
        }

        let cur = tool.execute(&make_ctx(), TodoArgs::Current).await.unwrap();
        match cur {
            TodoOutput::Current { todo } => assert!(todo.is_none()),
            _ => panic!("expected Current"),
        }
    }

    #[tokio::test]
    async fn too_many_skills_rejected() {
        let (tool, _dir) = make_tool();
        let sess = session("sess-skills");
        let host = NullToolHost;
        let ws = tempdir().unwrap();
        let ctx = ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path()));
        let err = tool
            .execute(
                &ctx,
                TodoArgs::Add {
                    title: "x".into(),
                    content: "x details".into(),
                    skills: vec!["a".into(), "b".into(), "c".into(), "d".into()],
                    context: None,
                },
            )
            .await
            .expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn done_alias_rejects_non_completed_status() {
        // execute_finish 内部约束：done 别名只能 completed。
        let dir = tempdir().unwrap();
        let path = dir.path().join("todos.json");
        let err = execute_finish(&path, None, TodoStatus::Failed, "x".into(), None, true)
            .await
            .expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[test]
    fn parse_add_cli_collects_repeated_skills() {
        let tokens = vec![
            "add".to_string(),
            "do the thing".into(),
            "--content".into(),
            "do the thing details".into(),
            "--skill".into(),
            "docs".into(),
            "--skill".into(),
            "code-rust".into(),
            "--context".into(),
            "why".into(),
        ];
        let json = parse_todo_cli_tokens(&tokens).unwrap();
        assert_eq!(json["op"], "add");
        assert_eq!(json["title"], "do the thing");
        assert_eq!(json["content"], "do the thing details");
        assert_eq!(json["skills"][0], "docs");
        assert_eq!(json["skills"][1], "code-rust");
        assert_eq!(json["context"], "why");
    }

    #[test]
    fn parse_add_cli_requires_content() {
        let tokens = vec!["add".to_string(), "do the thing".into()];
        let err = parse_todo_cli_tokens(&tokens).expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[test]
    fn parse_failed_cli_accepts_report() {
        let tokens = vec![
            "failed".to_string(),
            "upstream failed".into(),
            "--report".into(),
            "full failure report".into(),
        ];
        let json = parse_todo_cli_tokens(&tokens).unwrap();
        assert_eq!(json["op"], "failed");
        assert_eq!(json["summary"], "upstream failed");
        assert_eq!(json["report"], "full failure report");
    }

    #[test]
    fn parse_finish_requires_status() {
        let tokens = vec!["finish".to_string(), "summary".into()];
        let err = parse_todo_cli_tokens(&tokens).expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[test]
    fn parse_show_optional_id() {
        let json = parse_todo_cli_tokens(&["show".to_string()]).unwrap();
        assert!(json["todo_id"].is_null());
        let json = parse_todo_cli_tokens(&["show".to_string(), "T02".to_string()]).unwrap();
        assert_eq!(json["todo_id"], "T02");
    }

    #[tokio::test]
    async fn delegate_task_registers_workspace_entry() {
        let host = NullToolHost;
        let sess = session("sess-del");
        let ws = tempdir().unwrap();
        let ctx = ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path()));
        let tool = DelegateTaskTool::with_stub();
        let out = tool
            .execute(
                &ctx,
                DelegateTaskArgs {
                    task: "please review".into(),
                    to: Some("reviewer".into()),
                    context: Some("v2 protocol".into()),
                },
            )
            .await
            .unwrap();
        assert!(out.task_id.starts_with("tsk_"));
        let path = tasks_path(ws.path());
        let entries = read_tasks(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].task_id, out.task_id);
        assert_eq!(entries[0].origin_session_id, "sess-del");
        assert_eq!(entries[0].purpose, "v2 protocol");
        assert_eq!(entries[0].label.as_deref(), Some("reviewer"));
    }

    #[tokio::test]
    async fn clean_drops_pending_keeps_terminal_and_compacts_order() {
        let (tool, _dir) = make_tool();
        let sess = session("sess-clean");
        let host = NullToolHost;
        let ws = tempdir().unwrap();
        let make_ctx = || ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path()));

        // T01 → completed, T02 → pending, T03 → pending
        tool.execute(
            &make_ctx(),
            TodoArgs::Add {
                title: "first".into(),
                content: "first details".into(),
                skills: vec![],
                context: None,
            },
        )
        .await
        .unwrap();
        tool.execute(
            &make_ctx(),
            TodoArgs::Done {
                summary: "done".into(),
                report: None,
                report_file: None,
                todo_id: None,
            },
        )
        .await
        .unwrap();
        tool.execute(
            &make_ctx(),
            TodoArgs::Add {
                title: "second".into(),
                content: "second details".into(),
                skills: vec![],
                context: None,
            },
        )
        .await
        .unwrap();
        tool.execute(
            &make_ctx(),
            TodoArgs::Add {
                title: "third".into(),
                content: "third details".into(),
                skills: vec![],
                context: None,
            },
        )
        .await
        .unwrap();

        let cleaned = tool.execute(&make_ctx(), TodoArgs::Clean).await.unwrap();
        match cleaned {
            TodoOutput::Clean { removed, kept } => {
                assert_eq!(removed, vec!["T02".to_string(), "T03".to_string()]);
                assert_eq!(kept, 1);
            }
            _ => panic!("expected Clean"),
        }

        // 终态 T01 还在，没有未完成项，current = None。
        let cur = tool.execute(&make_ctx(), TodoArgs::Current).await.unwrap();
        match cur {
            TodoOutput::Current { todo } => assert!(todo.is_none()),
            _ => panic!(),
        }

        // 后续 addTodo 接着 baseline 重建，new id 从 T02 起（按 list 长度）。
        let out = tool
            .execute(
                &make_ctx(),
                TodoArgs::Add {
                    title: "replanned".into(),
                    content: "replanned details".into(),
                    skills: vec![],
                    context: None,
                },
            )
            .await
            .unwrap();
        match out {
            TodoOutput::Add {
                todo_id,
                order_index,
                is_current,
                ..
            } => {
                assert_eq!(todo_id, "T02");
                assert_eq!(order_index, 1);
                assert!(is_current);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_clean_takes_no_args() {
        let json = parse_todo_cli_tokens(&["clean".to_string()]).unwrap();
        assert_eq!(json["op"], "clean");
        let err = parse_todo_cli_tokens(&["clean".to_string(), "x".to_string()])
            .expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn current_returns_none_when_empty() {
        let (tool, _dir) = make_tool();
        let sess = session("sess-empty");
        let host = NullToolHost;
        let ws = tempdir().unwrap();
        let out = tool
            .execute(
                &ToolCtx::new(&sess, &host).with_shell_cwd(Some(ws.path())),
                TodoArgs::Current,
            )
            .await
            .unwrap();
        match out {
            TodoOutput::Current { todo } => assert!(todo.is_none()),
            _ => panic!(),
        }
    }

    // Suppress unused-imports warnings in test mode for helpers that may not be
    // used by every test build.
    #[allow(dead_code)]
    fn _force_use(t: &TodoTool) {
        run_with_cwd(t, &session("s"), &NullToolHost, None, |_| async { () });
    }
}
