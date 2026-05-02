//! 服务进程内的本地状态：Workflow 定义表、Run 表、事件流、Amendment 表，以及
//! 给 orchestrator 用的 task tracker 包装。
//!
//! 这一层只负责"workflow service 私有的、task_manager 不理解的"语义状态
//! （[doc/workflow/workflow service.md §0.1 第 4 条](../../../../doc/workflow/workflow%20service.md)）：
//! Definition 编译态、Run 节点拓扑/输出/活跃集、事件流、Amendment 版本链。
//! Run / Step / Thunk 的 status / progress / payload 由 orchestrator 通过
//! [`WorkflowTaskTracker`] 写到 task_manager，本层不重复持久化。
//!
//! 一期实现使用进程内存：足够跑通 §10 的验收标准（提交 / 启动 / 推进 / 人类
//! 介入 / Amendment / 事件回放）。`docs §5.1` 提到的 SQLite/sled 持久化是
//! 后续提交的工作。

use async_trait::async_trait;
use buckyos_api::{TaskManagerClient, WorkflowDefinition};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    AnalysisReport, CompiledWorkflow, EventEnvelope, NoopTaskTracker, TaskManagerTaskTracker,
    WorkflowError, WorkflowResult, WorkflowRun, WorkflowTaskTracker,
};

/// Workflow Definition / Run 的所有者三元组。映射到 task_manager / ACL 上的
/// `(user_id, app_id)`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Owner {
    pub user_id: String,
    pub app_id: String,
}

impl Owner {
    pub fn new(user_id: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            app_id: app_id.into(),
        }
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        Some(Self {
            user_id: map
                .get("user_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            app_id: map
                .get("app_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "user_id": self.user_id,
            "app_id": self.app_id,
        })
    }
}

/// `workflow.submit_definition` 把 Definition 写入服务 = `Active`；
/// `workflow.archive_definition` 改为 `Archived`，仍然可被已有 Run 引用，
/// 但禁止再创建 Run。`Draft` 给后续 dry-run/草稿场景预留。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionStatus {
    Draft,
    Active,
    Archived,
}

#[derive(Debug, Clone)]
pub struct DefinitionRecord {
    pub id: String,
    pub schema_version: String,
    pub name: String,
    pub owner: Owner,
    pub definition: WorkflowDefinition,
    pub compiled: CompiledWorkflow,
    pub analysis: AnalysisReport,
    pub status: DefinitionStatus,
    pub version: u32,
    pub definition_hash: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl DefinitionRecord {
    /// 给 RPC 响应用：把 record 投影成 §2.1 描述的 wire JSON。
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "schema_version": self.schema_version,
            "name": self.name,
            "owner": self.owner.to_value(),
            "definition": self.definition,
            "compiled": self.compiled,
            "analysis": self.analysis,
            "status": self.status,
            "version": self.version,
            "tags": self.tags,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }

    /// 不带 `definition` / `compiled` 这种大字段的 list 视图。
    pub fn to_summary_value(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "schema_version": self.schema_version,
            "name": self.name,
            "owner": self.owner.to_value(),
            "status": self.status,
            "version": self.version,
            "tags": self.tags,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "warnings": self.analysis.warnings.len(),
            "errors": self.analysis.errors.len(),
        })
    }
}

/// 服务侧 Workflow Definition 表。
#[derive(Default)]
pub struct DefinitionStore {
    inner: RwLock<DefinitionInner>,
}

#[derive(Default)]
struct DefinitionInner {
    by_id: HashMap<String, Arc<DefinitionRecord>>,
    /// (owner, name) -> 同名 definition 的最大 version。
    versions: HashMap<(Owner, String), u32>,
    /// (owner, name, definition_hash) -> id，用来给 §3.1 的幂等约束做查重。
    hashes: HashMap<(Owner, String, String), String>,
}

impl DefinitionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 按 `(owner, name, hash(definition))` 幂等：同样的 owner + name + 内容
    /// 第二次提交直接返回上一次的 record。
    pub async fn upsert(
        &self,
        owner: Owner,
        definition: WorkflowDefinition,
        compiled: CompiledWorkflow,
        analysis: AnalysisReport,
        tags: Vec<String>,
    ) -> Arc<DefinitionRecord> {
        let now = Utc::now().timestamp();
        let hash = definition_hash(&definition);
        let mut guard = self.inner.write().await;
        if let Some(existing_id) = guard
            .hashes
            .get(&(owner.clone(), definition.name.clone(), hash.clone()))
            .cloned()
        {
            if let Some(existing) = guard.by_id.get(&existing_id).cloned() {
                return existing;
            }
        }

        let next_version = guard
            .versions
            .get(&(owner.clone(), definition.name.clone()))
            .copied()
            .unwrap_or(0)
            + 1;
        let id = format!("wf-{}", Uuid::new_v4());
        let record = Arc::new(DefinitionRecord {
            id: id.clone(),
            schema_version: definition.schema_version.clone(),
            name: definition.name.clone(),
            owner: owner.clone(),
            definition,
            compiled,
            analysis,
            status: DefinitionStatus::Active,
            version: next_version,
            definition_hash: hash.clone(),
            tags,
            created_at: now,
            updated_at: now,
        });
        guard
            .versions
            .insert((owner.clone(), record.name.clone()), next_version);
        guard
            .hashes
            .insert((owner, record.name.clone(), hash), id.clone());
        guard.by_id.insert(id, record.clone());
        record
    }

    pub async fn get_by_id(&self, id: &str) -> Option<Arc<DefinitionRecord>> {
        self.inner.read().await.by_id.get(id).cloned()
    }

    pub async fn list(
        &self,
        owner: Option<&Owner>,
        status: Option<DefinitionStatus>,
        tag: Option<&str>,
    ) -> Vec<Arc<DefinitionRecord>> {
        let guard = self.inner.read().await;
        let mut out: Vec<_> = guard
            .by_id
            .values()
            .filter(|record| owner.map(|o| record.owner == *o).unwrap_or(true))
            .filter(|record| status.map(|s| record.status == s).unwrap_or(true))
            .filter(|record| tag.map(|t| record.tags.iter().any(|x| x == t)).unwrap_or(true))
            .cloned()
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    pub async fn archive(&self, id: &str) -> Option<Arc<DefinitionRecord>> {
        let mut guard = self.inner.write().await;
        let existing = guard.by_id.get(id).cloned()?;
        let updated = Arc::new(DefinitionRecord {
            status: DefinitionStatus::Archived,
            updated_at: Utc::now().timestamp(),
            ..(*existing).clone()
        });
        guard.by_id.insert(id.to_string(), updated.clone());
        Some(updated)
    }
}

/// 用 (workflow name, definition JSON) 算确定性 hash，是 §3.1 幂等键的一部分。
fn definition_hash(definition: &WorkflowDefinition) -> String {
    let bytes = serde_json::to_vec(definition).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Amendment 状态机：§3.4 的三个 RPC（submit / approve / reject）落到这里。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AmendmentStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone)]
pub struct AmendmentRecord {
    pub id: String,
    pub plan_version: u32,
    pub patch: Value,
    pub status: AmendmentStatus,
    pub submitted_by: String,
    pub submitted_at: i64,
    pub decided_by: Option<String>,
    pub decided_at: Option<i64>,
    pub reason: Option<String>,
}

impl AmendmentRecord {
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "plan_version": self.plan_version,
            "patch": self.patch,
            "status": self.status,
            "submitted_by": self.submitted_by,
            "submitted_at": self.submitted_at,
            "decided_by": self.decided_by,
            "decided_at": self.decided_at,
            "reason": self.reason,
        })
    }
}

/// 一个 Run 的本地视图。事件流 / amendments 是 workflow service 私有的；
/// `run` 字段镜像 task_manager 中对应 task 树的 high-level 状态（见 §0.1 第 5 条）。
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub run: WorkflowRun,
    pub workflow_id: String,
    pub owner: Owner,
    pub events: Vec<EventEnvelope>,
    pub amendments: Vec<AmendmentRecord>,
    pub callback_url: Option<String>,
}

impl RunRecord {
    pub fn append_events(&mut self, events: &[EventEnvelope]) {
        self.events.extend_from_slice(events);
    }

    pub fn events_since(&self, since_seq: u64, limit: usize) -> Vec<EventEnvelope> {
        self.events
            .iter()
            .filter(|event| event.seq > since_seq)
            .take(limit)
            .cloned()
            .collect()
    }
}

/// 一个 Run 的并发入口：单 Run 串行（per-run lock + 顺序事件 seq，§5.2）。
pub struct RunHandle {
    pub workflow_id: String,
    pub owner: Owner,
    pub state: Mutex<RunRecord>,
}

#[derive(Default)]
pub struct RunStore {
    runs: RwLock<HashMap<String, Arc<RunHandle>>>,
}

impl RunStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, record: RunRecord) -> Arc<RunHandle> {
        let run_id = record.run.run_id.clone();
        let workflow_id = record.workflow_id.clone();
        let owner = record.owner.clone();
        let handle = Arc::new(RunHandle {
            workflow_id,
            owner,
            state: Mutex::new(record),
        });
        self.runs.write().await.insert(run_id, handle.clone());
        handle
    }

    pub async fn get(&self, run_id: &str) -> Option<Arc<RunHandle>> {
        self.runs.read().await.get(run_id).cloned()
    }

    pub async fn list(
        &self,
        owner: Option<&Owner>,
        workflow_id: Option<&str>,
    ) -> Vec<Arc<RunHandle>> {
        let guard = self.runs.read().await;
        guard
            .values()
            .filter(|handle| owner.map(|o| handle.owner == *o).unwrap_or(true))
            .filter(|handle| {
                workflow_id
                    .map(|wf| handle.workflow_id == wf)
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }
}

/// 给 orchestrator 用的 [`WorkflowTaskTracker`] 包装。
///
/// task_manager 客户端可用时把 Run / Step / Map shard / Thunk 同步上去
/// （§6.3）；不可用时退化为 noop——参考 §1.2 的"task_manager 不可用时
/// workflow 仍可继续推进 adapter 主路径"约定。
pub struct ServiceTracker {
    inner: TrackerKind,
}

enum TrackerKind {
    Noop(NoopTaskTracker),
    TaskManager(TaskManagerTaskTracker),
}

impl ServiceTracker {
    pub fn noop() -> Self {
        Self {
            inner: TrackerKind::Noop(NoopTaskTracker::default()),
        }
    }

    pub fn from_task_manager(
        client: Arc<TaskManagerClient>,
        user_id: impl Into<String>,
        app_id: impl Into<String>,
    ) -> Self {
        Self {
            inner: TrackerKind::TaskManager(TaskManagerTaskTracker::new(client, user_id, app_id)),
        }
    }

    pub fn is_task_manager_backed(&self) -> bool {
        matches!(self.inner, TrackerKind::TaskManager(_))
    }
}

#[async_trait]
impl WorkflowTaskTracker for ServiceTracker {
    async fn sync_run(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        match &self.inner {
            TrackerKind::Noop(t) => t.sync_run(run).await,
            TrackerKind::TaskManager(t) => t.sync_run(run).await,
        }
    }

    async fn sync_step(
        &self,
        run: &WorkflowRun,
        step: &crate::StepTaskView,
    ) -> WorkflowResult<()> {
        match &self.inner {
            TrackerKind::Noop(t) => t.sync_step(run, step).await,
            TrackerKind::TaskManager(t) => t.sync_step(run, step).await,
        }
    }

    async fn sync_map_shard(
        &self,
        run: &WorkflowRun,
        shard: &crate::MapShardTaskView,
    ) -> WorkflowResult<()> {
        match &self.inner {
            TrackerKind::Noop(t) => t.sync_map_shard(run, shard).await,
            TrackerKind::TaskManager(t) => t.sync_map_shard(run, shard).await,
        }
    }

    async fn sync_thunk(
        &self,
        run: &WorkflowRun,
        thunk: &crate::ThunkTaskView,
    ) -> WorkflowResult<()> {
        match &self.inner {
            TrackerKind::Noop(t) => t.sync_thunk(run, thunk).await,
            TrackerKind::TaskManager(t) => t.sync_thunk(run, thunk).await,
        }
    }

    async fn report_step_validation_error(
        &self,
        run: &WorkflowRun,
        node_id: &str,
        message: &str,
    ) -> WorkflowResult<()> {
        match &self.inner {
            TrackerKind::Noop(t) => t.report_step_validation_error(run, node_id, message).await,
            TrackerKind::TaskManager(t) => {
                t.report_step_validation_error(run, node_id, message).await
            }
        }
    }
}

/// 把 [`WorkflowError`] 翻译成 §3.x RPC 想要的 `(code, message, detail)`，
/// 给 server.rs 拼 RPC error / error payload 用。
pub fn workflow_error_payload(err: &WorkflowError) -> (String, String, Option<Value>) {
    match err {
        WorkflowError::Analysis(report) => (
            "analysis_failed".to_string(),
            err.to_string(),
            Some(serde_json::to_value(report).unwrap_or(Value::Null)),
        ),
        WorkflowError::NodeNotFound(node) => (
            "node_not_found".to_string(),
            err.to_string(),
            Some(serde_json::json!({ "node_id": node })),
        ),
        WorkflowError::InvalidHumanAction { node_id, action } => (
            "invalid_human_action".to_string(),
            err.to_string(),
            Some(serde_json::json!({ "node_id": node_id, "action": action })),
        ),
        WorkflowError::RollbackBlocked(node) => (
            "rollback_blocked".to_string(),
            err.to_string(),
            Some(serde_json::json!({ "blocked_by": node })),
        ),
        WorkflowError::NodeNotSkippable(node) => (
            "node_not_skippable".to_string(),
            err.to_string(),
            Some(serde_json::json!({ "node_id": node })),
        ),
        WorkflowError::NodeNotWaitingHuman(node) => (
            "node_not_waiting_human".to_string(),
            err.to_string(),
            Some(serde_json::json!({ "node_id": node })),
        ),
        WorkflowError::UnresolvedSemanticExecutor { node_id, executor } => (
            "unresolved_semantic_executor".to_string(),
            err.to_string(),
            Some(serde_json::json!({ "node_id": node_id, "executor": executor })),
        ),
        WorkflowError::ExecutorNamespaceNotImplemented {
            node_id,
            executor,
            namespace,
        } => (
            "executor_namespace_not_implemented".to_string(),
            err.to_string(),
            Some(serde_json::json!({
                "node_id": node_id,
                "executor": executor,
                "namespace": namespace,
            })),
        ),
        _ => ("workflow_error".to_string(), err.to_string(), None),
    }
}
