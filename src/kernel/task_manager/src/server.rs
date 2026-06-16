use crate::download_executor::{
    build_download_task_data, build_download_task_name, infer_objid_from_url,
    merge_download_source_patch, shared_download_executor, should_enqueue_download_task,
    spec_from_task, task_has_download_url, task_has_objid, DownloadTaskStore, DOWNLOAD_TASK_TYPE,
};
use crate::task::{new_task, Task, TaskNote, TaskScope, TaskStatus};
use crate::task_db::TaskDb;
use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_http_server::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::*;
use ndn_lib::ObjId;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

// Keep the inlined data small enough that the full event (envelope + task
// fields + data) fits in a single shared-ringbuffer slot (SLOT_DATA_SIZE =
// 2048). Envelope + the 13 top-level task fields take ~400-700 bytes, so
// 1300 leaves headroom.
const TASK_EVENT_DATA_INLINE_LIMIT_BYTES: usize = 1300;
const TASK_EVENT_RATE_LIMIT: Duration = Duration::from_secs(1);
const DEFAULT_TASK_CLAIM_LEASE: Duration = Duration::from_secs(60);
const MAX_TASK_CLAIM_LEASE: Duration = Duration::from_secs(60 * 60);
const MIN_TASK_CLAIM_LEASE: Duration = Duration::from_secs(1);
const STALE_TASK_CLAIM_SWEEP_INTERVAL: Duration = Duration::from_secs(30);
const STALE_TASK_CLAIM_FAILED_MESSAGE: &str = "Task claim lease expired";
const STALE_TASK_CLAIM_MANUAL_MESSAGE: &str =
    "Task claim lease expired; waiting for manual recovery";
const NODE_DAEMON_RUNNER_APP_ID: &str = "node-daemon";
const TASK_TYPE_SCHEDULER_DISPATCH_THUNK: &str = "scheduler.dispatch_thunk";
const TASK_TYPE_AGENT_DELEGATE: &str = "agent.delegate";

#[derive(Clone, Copy)]
enum TaskChangeKind {
    Status,
    Error,
    Data,
    Progress,
}

impl TaskChangeKind {
    fn as_str(self) -> &'static str {
        match self {
            TaskChangeKind::Status => "status",
            TaskChangeKind::Error => "error",
            TaskChangeKind::Data => "data",
            TaskChangeKind::Progress => "progress",
        }
    }

    fn always_emit(self) -> bool {
        matches!(self, TaskChangeKind::Status | TaskChangeKind::Error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReclaimPolicy {
    Retry,
    Fail,
    Manual,
}

impl ReclaimPolicy {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "retry" | "requeue" => Some(Self::Retry),
            "fail" | "failed" => Some(Self::Fail),
            "manual" | "manual_intervention" | "manual-intervention" => Some(Self::Manual),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub user_id: String,
    pub app_id: String,
}

impl RequestContext {
    pub fn empty() -> Self {
        Self {
            user_id: "".to_string(),
            app_id: "".to_string(),
        }
    }
}

fn request_context_from_source(user_id: Option<&str>, app_id: Option<&str>) -> RequestContext {
    RequestContext {
        user_id: user_id.unwrap_or_default().to_string(),
        app_id: app_id.unwrap_or_default().to_string(),
    }
}

fn request_context_from_rpc(ctx: &RPCContext) -> RequestContext {
    let Some(token) = ctx.token.as_ref() else {
        return RequestContext::empty();
    };
    let Ok(session_token) = RPCSessionToken::from_string(token.as_str()) else {
        return RequestContext::empty();
    };
    let Ok((user_id, app_id)) = session_token.get_subs() else {
        return RequestContext::empty();
    };
    RequestContext { user_id, app_id }
}

fn request_context_from_source_or_rpc(
    user_id: Option<&str>,
    app_id: Option<&str>,
    ctx: &RPCContext,
) -> RequestContext {
    let mut request_ctx = request_context_from_source(user_id, app_id);
    if !request_ctx.user_id.is_empty() && !request_ctx.app_id.is_empty() {
        return request_ctx;
    }

    let rpc_ctx = request_context_from_rpc(ctx);
    if request_ctx.user_id.is_empty() {
        request_ctx.user_id = rpc_ctx.user_id;
    }
    if request_ctx.app_id.is_empty() {
        request_ctx.app_id = rpc_ctx.app_id;
    }
    request_ctx
}

#[derive(Clone)]
struct TaskManagerService {
    kevent_client: KEventClient,
    db: Arc<TaskDb>,
    last_event_at: Arc<StdMutex<HashMap<i64, Instant>>>,
}

impl TaskManagerService {
    pub fn new(db: Arc<TaskDb>) -> Self {
        TaskManagerService {
            kevent_client: KEventClient::new_full(TASK_MANAGER_SERVICE_NAME, None),
            db,
            last_event_at: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    fn is_system_app(app_id: &str) -> bool {
        app_id == "kernel" || app_id == "system"
    }

    fn is_system_context(ctx: &RequestContext) -> bool {
        !ctx.app_id.trim().is_empty() && Self::is_system_app(ctx.app_id.as_str())
    }

    fn can_manage_runner(ctx: &RequestContext, runner: &str) -> bool {
        if ctx.user_id.trim().is_empty() || ctx.app_id.trim().is_empty() {
            return false;
        }
        if Self::is_system_context(ctx) {
            return true;
        }
        let runner = runner.trim();
        if runner.is_empty() {
            return false;
        }
        if ctx.app_id == runner {
            return true;
        }
        ctx.app_id == NODE_DAEMON_RUNNER_APP_ID
            && (ctx.user_id == runner || ctx.user_id == "kernel")
    }

    fn task_event_id(task_id: i64) -> String {
        format!("/task_mgr/{}", task_id)
    }

    fn root_event_id(root_id: &str) -> Option<String> {
        let trimmed = root_id.trim();
        if trimmed.is_empty() || trimmed.contains('/') {
            return None;
        }
        let event_id = format!("/task_mgr/{}", trimmed);
        validate_eventid(event_id.as_str()).ok()?;
        Some(event_id)
    }

    fn runner_task_ready_event_id(runner: &str) -> Option<String> {
        let trimmed = runner.trim();
        if trimmed.is_empty() || trimmed.contains('/') {
            return None;
        }
        let event_id = format!("/task_mgr/runner/{}/task_ready", trimmed);
        validate_eventid(event_id.as_str()).ok()?;
        Some(event_id)
    }

    fn now_ts() -> u64 {
        chrono::Utc::now().timestamp().max(0) as u64
    }

    fn resolve_claim_lease(lease_ms: Option<u64>) -> Duration {
        let requested = lease_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_TASK_CLAIM_LEASE);
        requested.clamp(MIN_TASK_CLAIM_LEASE, MAX_TASK_CLAIM_LEASE)
    }

    fn build_claim_window(lease_ms: Option<u64>) -> (u64, u64) {
        let now = Self::now_ts();
        let lease = Self::resolve_claim_lease(lease_ms);
        let lease_until = now.saturating_add(lease.as_secs());
        (now, lease_until)
    }

    fn configured_reclaim_policy(data: &Value) -> Option<ReclaimPolicy> {
        let keys = ["reclaim_policy", "reclaimPolicy"];
        for key in keys {
            if let Some(policy) = data
                .get(key)
                .and_then(Value::as_str)
                .and_then(ReclaimPolicy::from_str)
            {
                return Some(policy);
            }
        }

        let nested_paths = [
            ("task_claim", "reclaim_policy"),
            ("task_claim", "reclaimPolicy"),
            ("taskClaim", "reclaim_policy"),
            ("taskClaim", "reclaimPolicy"),
            ("claim", "reclaim_policy"),
            ("claim", "reclaimPolicy"),
        ];
        for (object_key, policy_key) in nested_paths {
            if let Some(policy) = data
                .get(object_key)
                .and_then(|value| value.get(policy_key))
                .and_then(Value::as_str)
                .and_then(ReclaimPolicy::from_str)
            {
                return Some(policy);
            }
        }

        None
    }

    fn reclaim_policy_for_task(task: &Task) -> ReclaimPolicy {
        if let Some(policy) = Self::configured_reclaim_policy(&task.data) {
            return policy;
        }

        match task.task_type.as_str() {
            TASK_TYPE_SCHEDULER_DISPATCH_THUNK => ReclaimPolicy::Retry,
            TASK_TYPE_AGENT_DELEGATE => ReclaimPolicy::Manual,
            _ => ReclaimPolicy::Manual,
        }
    }

    async fn reclaim_stale_task_claims(
        &self,
        now: u64,
        runner: Option<&str>,
        source_method: &str,
    ) -> Result<RequeueStaleClaimsResult> {
        let stale_tasks = self
            .db
            .list_stale_claimed_tasks(now, runner)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let mut result = RequeueStaleClaimsResult {
            requeued_count: 0,
            failed_count: 0,
            manual_count: 0,
        };

        for before_task in stale_tasks {
            let policy = Self::reclaim_policy_for_task(&before_task);
            let changed = match policy {
                ReclaimPolicy::Retry => {
                    let changed = self
                        .db
                        .requeue_stale_claim(before_task.id, now)
                        .await
                        .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
                    if changed {
                        result.requeued_count += 1;
                    }
                    changed
                }
                ReclaimPolicy::Fail => {
                    let changed = self
                        .db
                        .fail_stale_claim(before_task.id, now, STALE_TASK_CLAIM_FAILED_MESSAGE)
                        .await
                        .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
                    if changed {
                        result.failed_count += 1;
                    }
                    changed
                }
                ReclaimPolicy::Manual => {
                    let changed = self
                        .db
                        .mark_stale_claim_manual(
                            before_task.id,
                            now,
                            STALE_TASK_CLAIM_MANUAL_MESSAGE,
                        )
                        .await
                        .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
                    if changed {
                        result.manual_count += 1;
                    }
                    changed
                }
            };

            if !changed {
                continue;
            }

            match self.load_task(before_task.id).await {
                Ok(after_task) => {
                    if let Some(kind) = Self::diff_kind(&before_task, &after_task) {
                        self.publish_task_changed_event(
                            &before_task,
                            &after_task,
                            kind,
                            source_method,
                        )
                        .await;
                    }
                    if after_task.status == TaskStatus::Pending {
                        self.publish_task_ready_event(&after_task, source_method)
                            .await;
                    }
                }
                Err(err) => {
                    warn!(
                        "task_mgr.reclaim_stale_task_claims failed to reload task {} for event publish: {}",
                        before_task.id, err
                    );
                }
            }
        }

        Ok(result)
    }

    fn start_stale_claim_sweeper(&self) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(STALE_TASK_CLAIM_SWEEP_INTERVAL);
            loop {
                interval.tick().await;
                let now = TaskManagerService::now_ts();
                match service
                    .reclaim_stale_task_claims(now, None, "stale_claim_sweep")
                    .await
                {
                    Ok(result)
                        if result.requeued_count > 0
                            || result.failed_count > 0
                            || result.manual_count > 0 =>
                    {
                        info!(
                            "task_mgr.stale_claim_sweep: requeued={} failed={} manual={}",
                            result.requeued_count, result.failed_count, result.manual_count
                        );
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!("task_mgr.stale_claim_sweep failed: {}", err);
                    }
                }
            }
        });
    }

    /// Returns true if the rate-limit gate is open and updates the last-emit
    /// timestamp. `Status` / `Error` kinds always pass and refresh the gate so
    /// a low-priority emit doesn't fire right after a critical one.
    fn check_and_update_rate_limit(&self, task_id: i64, kind: TaskChangeKind) -> bool {
        let now = Instant::now();
        let mut guard = self.last_event_at.lock().unwrap();
        if kind.always_emit() {
            guard.insert(task_id, now);
            return true;
        }
        match guard.get(&task_id) {
            Some(prev) if now.duration_since(*prev) < TASK_EVENT_RATE_LIMIT => false,
            _ => {
                guard.insert(task_id, now);
                true
            }
        }
    }

    fn build_event_payload(
        &self,
        before: &Task,
        after: &Task,
        kind: TaskChangeKind,
        source_method: &str,
    ) -> Value {
        let mut payload = json!({
            "task_id": after.id,
            "root_id": after.root_id,
            "parent_id": after.parent_id,
            "user_id": after.user_id,
            "app_id": after.app_id,
            "session_id": after.session_id,
            "task_type": after.task_type,
            "runner": after.runner,
            "from_status": before.status.to_string(),
            "to_status": after.status.to_string(),
            "progress": after.progress,
            "message": after.message,
            "updated_at": after.updated_at,
            "source_method": source_method,
            "change_kind": kind.as_str(),
        });

        let data_size = serde_json::to_vec(&after.data)
            .map(|v| v.len())
            .unwrap_or(0);
        let map = payload.as_object_mut().expect("payload is a json object");
        if data_size <= TASK_EVENT_DATA_INLINE_LIMIT_BYTES {
            map.insert("data".to_string(), after.data.clone());
        } else {
            map.insert("data_omitted".to_string(), Value::Bool(true));
            map.insert(
                "data_size".to_string(),
                Value::Number(serde_json::Number::from(data_size as u64)),
            );
        }
        payload
    }

    fn build_task_ready_payload(&self, task: &Task, source_method: &str) -> Value {
        json!({
            "event_kind": "task_ready",
            "task_id": task.id,
            "root_id": task.root_id,
            "parent_id": task.parent_id,
            "user_id": task.user_id,
            "app_id": task.app_id,
            "session_id": task.session_id,
            "task_type": task.task_type,
            "runner": task.runner,
            "status": task.status.to_string(),
            "created_at": task.created_at,
            "updated_at": task.updated_at,
            "source_method": source_method,
        })
    }

    async fn publish_task_ready_event(&self, task: &Task, source_method: &str) {
        if task.status != TaskStatus::Pending {
            return;
        }
        let Some(event_id) = Self::runner_task_ready_event_id(task.runner.as_str()) else {
            return;
        };
        let payload = self.build_task_ready_payload(task, source_method);
        if let Err(err) = self
            .kevent_client
            .pub_event(event_id.as_str(), payload)
            .await
        {
            warn!(
                "task_mgr.publish_task_ready_event failed: event_id={} task_id={} runner={} err={}",
                event_id, task.id, task.runner, err
            );
        }
    }

    /// Publish a task-changed event. Status / Error transitions always fire;
    /// Data / Progress changes are rate-limited per task_id at
    /// TASK_EVENT_RATE_LIMIT. Subscribers can listen on `/task_mgr/{task_id}`
    /// for a single task, or `/task_mgr/{root_id}` to receive every event in
    /// the subtree (root + descendants).
    async fn publish_task_changed_event(
        &self,
        before: &Task,
        after: &Task,
        kind: TaskChangeKind,
        source_method: &str,
    ) {
        if !self.check_and_update_rate_limit(after.id, kind) {
            return;
        }

        let payload = self.build_event_payload(before, after, kind, source_method);

        let task_event_id = Self::task_event_id(after.id);
        if let Err(err) = self
            .kevent_client
            .pub_event(task_event_id.as_str(), payload.clone())
            .await
        {
            warn!(
                "task_mgr.publish_task_changed_event failed: event_id={} task_id={} kind={} err={}",
                task_event_id,
                after.id,
                kind.as_str(),
                err
            );
        }

        // Also fan out to the root-id channel so subtree subscribers see
        // every descendant event. Root tasks (root_id == task_id) are not
        // republished — the per-task channel already covers them.
        if let Some(root_event_id) = Self::root_event_id(after.root_id.as_str()) {
            if root_event_id != task_event_id {
                if let Err(err) = self
                    .kevent_client
                    .pub_event(root_event_id.as_str(), payload)
                    .await
                {
                    warn!(
                        "task_mgr.publish_task_changed_event root fanout failed: event_id={} task_id={} err={}",
                        root_event_id, after.id, err
                    );
                }
            }
        }
    }

    fn diff_kind(before: &Task, after: &Task) -> Option<TaskChangeKind> {
        if before.status != after.status {
            return Some(TaskChangeKind::Status);
        }
        if (before.progress - after.progress).abs() > f32::EPSILON {
            return Some(TaskChangeKind::Progress);
        }
        if before.data != after.data {
            return Some(TaskChangeKind::Data);
        }
        if before.message != after.message {
            return Some(TaskChangeKind::Data);
        }
        None
    }

    fn can_read_task(&self, ctx: &RequestContext, task: &Task) -> bool {
        if ctx.user_id.is_empty() && ctx.app_id.is_empty() {
            return true;
        }
        if task.user_id.is_empty() {
            return task.app_id.is_empty() || task.app_id == ctx.app_id;
        }

        match task.permissions.read {
            TaskScope::Private => task.user_id == ctx.user_id && task.app_id == ctx.app_id,
            TaskScope::User => task.user_id == ctx.user_id,
            TaskScope::System => Self::is_system_app(ctx.app_id.as_str()),
        }
    }

    fn can_write_task(&self, ctx: &RequestContext, task: &Task) -> bool {
        if ctx.user_id.is_empty() && ctx.app_id.is_empty() {
            return true;
        }
        if task.user_id.is_empty() {
            return task.app_id.is_empty() || task.app_id == ctx.app_id;
        }

        match task.permissions.write {
            TaskScope::Private => task.user_id == ctx.user_id && task.app_id == ctx.app_id,
            TaskScope::User => task.user_id == ctx.user_id,
            TaskScope::System => Self::is_system_app(ctx.app_id.as_str()),
        }
    }

    async fn load_task(&self, id: i64) -> Result<Task> {
        let task = self
            .db
            .get_task(id)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        task.ok_or_else(|| RPCErrors::ReasonError(format!("Task {} not found", id)))
    }

    async fn list_download_tasks_for_request(
        &self,
        request_ctx: &RequestContext,
    ) -> Result<Vec<Task>> {
        let user_id =
            (!request_ctx.user_id.trim().is_empty()).then_some(request_ctx.user_id.as_str());
        let app_id = (!request_ctx.app_id.trim().is_empty()).then_some(request_ctx.app_id.as_str());
        self.db
            .list_tasks_filtered(
                app_id,
                None,
                Some(DOWNLOAD_TASK_TYPE),
                None,
                None,
                None,
                None,
                user_id,
            )
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))
    }

    async fn update_task_and_publish(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data_patch: Option<Value>,
        source_method: &'static str,
    ) -> std::result::Result<Task, String> {
        let before_task = self.load_task(id).await.map_err(|err| err.to_string())?;
        self.db
            .update_task(id, status, progress, message, data_patch)
            .await
            .map_err(|err| err.to_string())?;

        let after_task = self.load_task(id).await.map_err(|err| err.to_string())?;
        if let Some(kind) = Self::diff_kind(&before_task, &after_task) {
            self.publish_task_changed_event(&before_task, &after_task, kind, source_method)
                .await;
        }
        Ok(after_task)
    }

    async fn update_task_error_and_publish(
        &self,
        id: i64,
        error_message: &str,
        source_method: &'static str,
    ) -> std::result::Result<Task, String> {
        let before_task = self.load_task(id).await.map_err(|err| err.to_string())?;
        self.db
            .update_task_error(id, error_message)
            .await
            .map_err(|err| err.to_string())?;

        let after_task = self.load_task(id).await.map_err(|err| err.to_string())?;
        self.publish_task_changed_event(
            &before_task,
            &after_task,
            TaskChangeKind::Error,
            source_method,
        )
        .await;
        Ok(after_task)
    }
}

#[async_trait]
impl DownloadTaskStore for TaskManagerService {
    async fn load_task(&self, task_id: i64) -> std::result::Result<Task, String> {
        TaskManagerService::load_task(self, task_id)
            .await
            .map_err(|err| err.to_string())
    }

    async fn update_task(
        &self,
        task_id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data: Option<DownloadTaskData>,
        source_method: &'static str,
    ) -> std::result::Result<Task, String> {
        let data = data
            .map(|data| serde_json::to_value(data).map_err(|err| err.to_string()))
            .transpose()?;
        self.update_task_and_publish(task_id, status, progress, message, data, source_method)
            .await
    }

    async fn mark_failed(
        &self,
        task_id: i64,
        error_message: String,
        source_method: &'static str,
    ) -> std::result::Result<Task, String> {
        self.update_task_error_and_publish(task_id, error_message.as_str(), source_method)
            .await
    }
}

#[async_trait]
impl TaskManagerHandler for TaskManagerService {
    async fn handle_create_task(
        &self,
        name: &str,
        task_type: &str,
        data: Option<Value>,
        opts: CreateTaskOptions,
        user_id: &str,
        app_id: &str,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = request_context_from_source_or_rpc(Some(user_id), Some(app_id), &ctx);
        let permissions = opts.permissions.unwrap_or_default();
        let data = data.unwrap_or_else(|| json!({}));

        let mut task = new_task(
            name.to_string(),
            task_type.to_string(),
            opts.runner.clone().unwrap_or_default(),
            request_ctx.user_id.clone(),
            request_ctx.app_id.clone(),
            opts.session_id.clone().unwrap_or_default(),
            opts.parent_id,
            permissions,
            data,
        );

        if let Some(parent_id) = task.parent_id {
            let parent = self.load_task(parent_id).await?;
            if !self.can_write_task(&request_ctx, &parent) {
                return Err(RPCErrors::NoPermission(
                    "No permission to create subtasks".to_string(),
                ));
            }
            task.root_id = parent.root_id;
        } else if let Some(root_id) = opts
            .root_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
        {
            task.root_id = root_id;
        }

        let task_id = self
            .db
            .create_task(&task)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        if task.root_id.trim().is_empty() {
            let root_id = task_id.to_string();
            self.db
                .set_root_id(task_id, root_id.as_str())
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
            task.root_id = root_id;
        }

        task.id = task_id;
        self.publish_task_ready_event(&task, "create_task").await;
        Ok(task)
    }

    async fn handle_create_download_task(
        &self,
        download_url: &str,
        objid: Option<ObjId>,
        download_options: Option<Value>,
        parent_id: Option<i64>,
        mut opts: CreateTaskOptions,
        user_id: &str,
        app_id: &str,
        ctx: RPCContext,
    ) -> Result<TaskId> {
        let download_url = download_url.trim();
        if download_url.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "download_url is required".to_string(),
            ));
        }

        if opts.parent_id.is_none() {
            opts.parent_id = parent_id;
        }

        let request_ctx = request_context_from_source_or_rpc(Some(user_id), Some(app_id), &ctx);
        let resolved_objid = objid.or_else(|| infer_objid_from_url(download_url));
        let scoped_tasks = self.list_download_tasks_for_request(&request_ctx).await?;

        let existing_task = resolved_objid
            .as_ref()
            .and_then(|objid| {
                scoped_tasks
                    .iter()
                    .find(|task| task_has_objid(task, objid))
                    .cloned()
            })
            .or_else(|| {
                scoped_tasks
                    .iter()
                    .find(|task| task_has_download_url(task, download_url))
                    .cloned()
            });

        if let Some(existing_task) = existing_task {
            let mut task = existing_task;
            if let Some(data_patch) = merge_download_source_patch(
                &task.data,
                download_url,
                resolved_objid.as_ref(),
                download_options.as_ref(),
            ) {
                self.db
                    .update_task(task.id, None, None, None, Some(data_patch))
                    .await
                    .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
                task = self.load_task(task.id).await?;
            }

            if should_enqueue_download_task(&task) {
                if let Some(spec) = spec_from_task(&task) {
                    let _ = shared_download_executor()
                        .enqueue(Arc::new(self.clone()), spec)
                        .await;
                }
            }
            return Ok(task.id);
        }

        let task_name = build_download_task_name(download_url, resolved_objid.as_ref());
        let task_data =
            build_download_task_data(download_url, resolved_objid.as_ref(), download_options);

        let task = self
            .handle_create_task(
                task_name.as_str(),
                DOWNLOAD_TASK_TYPE,
                Some(task_data),
                opts,
                user_id,
                app_id,
                ctx.clone(),
            )
            .await?;

        if let Some(spec) = spec_from_task(&task) {
            let _ = shared_download_executor()
                .enqueue(Arc::new(self.clone()), spec)
                .await;
        }

        Ok(task.id)
    }

    async fn handle_get_task(&self, id: i64, _ctx: RPCContext) -> Result<Task> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_read_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to read task".to_string(),
            ));
        }

        Ok(task)
    }

    async fn handle_claim_task(
        &self,
        id: i64,
        runner: &str,
        lease_ms: Option<u64>,
        ctx: RPCContext,
    ) -> Result<ClaimTaskResult> {
        let runner = runner.trim();
        if runner.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "runner is required".to_string(),
            ));
        }

        let request_ctx = request_context_from_rpc(&ctx);
        let before_task = self.load_task(id).await?;
        if !Self::can_manage_runner(&request_ctx, runner) {
            return Err(RPCErrors::NoPermission(
                "No permission to claim task".to_string(),
            ));
        }
        if before_task.runner != runner || before_task.status != TaskStatus::Pending {
            return Ok(ClaimTaskResult {
                task: None,
                claim_token: None,
                lease_until: None,
            });
        }

        let claim_token = Uuid::new_v4().to_string();
        let (now, lease_until) = Self::build_claim_window(lease_ms);
        let claimed = self
            .db
            .claim_task_for_runner(id, runner, claim_token.as_str(), now, lease_until)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        if let Some(after_task) = claimed.as_ref() {
            self.publish_task_changed_event(
                &before_task,
                after_task,
                TaskChangeKind::Status,
                "claim_task",
            )
            .await;
        }
        if claimed.is_none() {
            return Ok(ClaimTaskResult {
                task: None,
                claim_token: None,
                lease_until: None,
            });
        }

        Ok(ClaimTaskResult {
            task: claimed,
            claim_token: Some(claim_token),
            lease_until: Some(lease_until),
        })
    }

    async fn handle_heartbeat_task_claim(
        &self,
        id: i64,
        claim_token: &str,
        lease_ms: Option<u64>,
        ctx: RPCContext,
    ) -> Result<HeartbeatTaskClaimResult> {
        let claim_token = claim_token.trim();
        if claim_token.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "claim_token is required".to_string(),
            ));
        }

        let request_ctx = request_context_from_rpc(&ctx);
        let task = self.load_task(id).await?;
        if !Self::can_manage_runner(&request_ctx, task.runner.as_str()) {
            return Err(RPCErrors::NoPermission(
                "No permission to heartbeat task claim".to_string(),
            ));
        }

        let (now, lease_until) = Self::build_claim_window(lease_ms);
        let extended = self
            .db
            .heartbeat_task_claim(id, claim_token, now, lease_until)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        if !extended {
            return Err(RPCErrors::ReasonError(format!(
                "Task claim {} is not active",
                id
            )));
        }
        Ok(HeartbeatTaskClaimResult { lease_until })
    }

    async fn handle_requeue_stale_task_claims(
        &self,
        now: Option<u64>,
        runner: Option<&str>,
        ctx: RPCContext,
    ) -> Result<RequeueStaleClaimsResult> {
        let now = now.unwrap_or_else(Self::now_ts);
        let runner = runner.map(str::trim).filter(|value| !value.is_empty());
        let request_ctx = request_context_from_rpc(&ctx);
        if let Some(runner) = runner {
            if !Self::can_manage_runner(&request_ctx, runner) {
                return Err(RPCErrors::NoPermission(
                    "No permission to requeue runner claims".to_string(),
                ));
            }
        } else if !Self::is_system_context(&request_ctx) {
            return Err(RPCErrors::NoPermission(
                "No permission to requeue all stale claims".to_string(),
            ));
        }
        self.reclaim_stale_task_claims(now, runner, "requeue_stale_task_claims")
            .await
    }

    async fn handle_add_task_note(
        &self,
        task_id: i64,
        note_type: Option<&str>,
        content: &str,
        data: Option<Value>,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        ctx: RPCContext,
    ) -> Result<TaskNote> {
        let content = content.trim();
        if content.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "note content is required".to_string(),
            ));
        }

        let request_ctx = request_context_from_source_or_rpc(source_user_id, source_app_id, &ctx);
        let task = self.load_task(task_id).await?;
        if !self.can_read_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to add task note".to_string(),
            ));
        }

        let now = chrono::Utc::now().timestamp().max(0) as u64;
        let mut note = TaskNote {
            id: 0,
            task_id,
            note_type: note_type
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("human")
                .to_string(),
            content: content.to_string(),
            data: data.unwrap_or_else(|| json!({})),
            author_user_id: request_ctx.user_id,
            author_app_id: request_ctx.app_id,
            created_at: now,
            updated_at: now,
        };

        let note_id = self
            .db
            .add_task_note(&note)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        note.id = note_id;
        Ok(note)
    }

    async fn handle_list_task_notes(
        &self,
        task_id: i64,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        ctx: RPCContext,
    ) -> Result<Vec<TaskNote>> {
        let request_ctx = request_context_from_source_or_rpc(source_user_id, source_app_id, &ctx);
        let task = self.load_task(task_id).await?;
        if !self.can_read_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to list task notes".to_string(),
            ));
        }

        self.db
            .list_task_notes(task_id)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))
    }

    async fn handle_list_tasks(
        &self,
        filter: TaskFilter,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        _ctx: RPCContext,
    ) -> Result<Vec<Task>> {
        let request_ctx = request_context_from_source(source_user_id, source_app_id);
        let tasks = self
            .db
            .list_tasks_filtered(
                filter.app_id.as_deref(),
                filter.session_id.as_deref(),
                filter.task_type.as_deref(),
                filter.runner.as_deref(),
                filter.status,
                filter.parent_id,
                filter.root_id.as_deref(),
                None,
            )
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let filtered = tasks
            .into_iter()
            .filter(|task| self.can_read_task(&request_ctx, task))
            .collect();

        Ok(filtered)
    }

    async fn handle_list_tasks_by_time_range(
        &self,
        app_id: Option<&str>,
        session_id: Option<&str>,
        task_type: Option<&str>,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        time_range: Range<u64>,
        _ctx: RPCContext,
    ) -> Result<Vec<Task>> {
        let request_ctx = request_context_from_source(source_user_id, source_app_id);
        let tasks = self
            .db
            .list_tasks_filtered(app_id, session_id, task_type, None, None, None, None, None)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let filtered = tasks
            .into_iter()
            .filter(|task| {
                task.created_at >= time_range.start
                    && task.created_at < time_range.end
                    && self.can_read_task(&request_ctx, task)
            })
            .collect();

        Ok(filtered)
    }

    async fn handle_update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data: Option<Value>,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let before_task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &before_task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        self.db
            .update_task(id, status, progress, message, data)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        match self.load_task(id).await {
            Ok(after_task) => {
                if let Some(kind) = Self::diff_kind(&before_task, &after_task) {
                    self.publish_task_changed_event(&before_task, &after_task, kind, "update_task")
                        .await;
                }
            }
            Err(err) => {
                warn!(
                    "task_mgr.update_task failed to reload task {} for event publish: {}",
                    id, err
                );
            }
        }
        Ok(())
    }

    async fn handle_cancel_task(&self, id: i64, recursive: bool, _ctx: RPCContext) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to cancel task".to_string(),
            ));
        }

        if recursive {
            let root_id = if task.root_id.trim().is_empty() {
                task.id.to_string()
            } else {
                task.root_id.clone()
            };

            let before_tasks = self
                .db
                .list_tasks_filtered(
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(root_id.as_str()),
                    None,
                )
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

            self.db
                .update_task_status_by_root_id(root_id.as_str(), TaskStatus::Canceled)
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

            for before_task in before_tasks
                .into_iter()
                .filter(|existing| existing.status != TaskStatus::Canceled)
            {
                match self.load_task(before_task.id).await {
                    Ok(after_task) => {
                        self.publish_task_changed_event(
                            &before_task,
                            &after_task,
                            TaskChangeKind::Status,
                            "cancel_task_recursive",
                        )
                        .await;
                    }
                    Err(err) => {
                        warn!(
                            "task_mgr.cancel_task recursive failed to reload task {} for event publish: {}",
                            before_task.id, err
                        );
                    }
                }
            }
        } else {
            let before_task = task.clone();
            self.db
                .update_task_status(id, TaskStatus::Canceled)
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

            if before_task.status != TaskStatus::Canceled {
                match self.load_task(id).await {
                    Ok(after_task) => {
                        self.publish_task_changed_event(
                            &before_task,
                            &after_task,
                            TaskChangeKind::Status,
                            "cancel_task",
                        )
                        .await;
                    }
                    Err(err) => {
                        warn!(
                            "task_mgr.cancel_task failed to reload task {} for event publish: {}",
                            id, err
                        );
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_get_subtasks(&self, parent_id: i64, _ctx: RPCContext) -> Result<Vec<Task>> {
        let request_ctx = request_context_from_source(None, None);
        let tasks = self
            .db
            .list_tasks_filtered(None, None, None, None, None, Some(parent_id), None, None)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let filtered = tasks
            .into_iter()
            .filter(|task| self.can_read_task(&request_ctx, task))
            .collect();
        Ok(filtered)
    }

    async fn handle_update_task_status(
        &self,
        id: i64,
        status: TaskStatus,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let before_task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &before_task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        self.db
            .update_task_status(id, status)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        if before_task.status != status {
            match self.load_task(id).await {
                Ok(after_task) => {
                    self.publish_task_changed_event(
                        &before_task,
                        &after_task,
                        TaskChangeKind::Status,
                        "update_task_status",
                    )
                    .await;
                }
                Err(err) => {
                    warn!(
                        "task_mgr.update_task_status failed to reload task {} for event publish: {}",
                        id, err
                    );
                }
            }
        }
        Ok(())
    }

    async fn handle_update_task_progress(
        &self,
        id: i64,
        completed_items: u64,
        total_items: u64,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let before_task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &before_task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let progress = if total_items > 0 {
            (completed_items as f32 / total_items as f32) * 100.0
        } else {
            0.0
        };

        self.db
            .update_task_progress(id, progress, completed_items as i32, total_items as i32)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        match self.load_task(id).await {
            Ok(after_task) => {
                self.publish_task_changed_event(
                    &before_task,
                    &after_task,
                    TaskChangeKind::Progress,
                    "update_task_progress",
                )
                .await;
            }
            Err(err) => {
                warn!(
                    "task_mgr.update_task_progress failed to reload task {} for event publish: {}",
                    id, err
                );
            }
        }
        Ok(())
    }

    async fn handle_update_task_error(
        &self,
        id: i64,
        error_message: &str,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let before_task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &before_task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        self.db
            .update_task_error(id, error_message)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        match self.load_task(id).await {
            Ok(after_task) => {
                self.publish_task_changed_event(
                    &before_task,
                    &after_task,
                    TaskChangeKind::Error,
                    "update_task_error",
                )
                .await;
            }
            Err(err) => {
                warn!(
                    "task_mgr.update_task_error failed to reload task {} for event publish: {}",
                    id, err
                );
            }
        }
        Ok(())
    }

    async fn handle_update_task_data(&self, id: i64, data: Value, _ctx: RPCContext) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let before_task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &before_task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let data_str =
            serde_json::to_string(&data).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        self.db
            .update_task_data(id, data_str.as_str())
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        match self.load_task(id).await {
            Ok(after_task) => {
                if before_task.data != after_task.data {
                    self.publish_task_changed_event(
                        &before_task,
                        &after_task,
                        TaskChangeKind::Data,
                        "update_task_data",
                    )
                    .await;
                }
            }
            Err(err) => {
                warn!(
                    "task_mgr.update_task_data failed to reload task {} for event publish: {}",
                    id, err
                );
            }
        }
        Ok(())
    }

    async fn handle_delete_task(&self, id: i64, _ctx: RPCContext) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to delete task".to_string(),
            ));
        }

        self.db
            .delete_task(id)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_delete_tasks_by_session(
        &self,
        session_id: &str,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        _ctx: RPCContext,
    ) -> Result<u64> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "session_id is required".to_string(),
            ));
        }

        let request_ctx = request_context_from_source(source_user_id, source_app_id);
        let tasks = self
            .db
            .list_tasks_filtered(None, Some(session_id), None, None, None, None, None, None)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        if let Some(task) = tasks
            .iter()
            .find(|task| !self.can_write_task(&request_ctx, task))
        {
            return Err(RPCErrors::NoPermission(format!(
                "No permission to delete task {} in session {}",
                task.id, session_id
            )));
        }

        self.db
            .delete_tasks_by_session_id(session_id)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))
    }
}
pub struct TaskManagerHttpServer<T: TaskManagerHandler> {
    rpc_handler: buckyos_api::TaskManagerServerHandler<T>,
}

impl<T: TaskManagerHandler> TaskManagerHttpServer<T> {
    pub fn new(handler: T) -> Self {
        Self {
            rpc_handler: buckyos_api::TaskManagerServerHandler::new(handler),
        }
    }
}

#[async_trait]
impl<T: TaskManagerHandler + 'static> HttpServer for TaskManagerHttpServer<T> {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, &self.rpc_handler).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        "task-manager-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_task_manager_service() -> Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        TASK_MANAGER_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    if let Err(err) = runtime.login().await {
        error!("task manager service login to system failed! err:{:?}", err);
        return Err(RPCErrors::ReasonError(format!(
            "task manager login to system failed! err:{:?}",
            err
        )));
    }
    runtime
        .set_main_service_port(TASK_MANAGER_SERVICE_MAIN_PORT)
        .await;
    set_buckyos_api_runtime(runtime).map_err(|err| {
        RPCErrors::ReasonError(format!("register task manager runtime failed: {}", err))
    })?;

    let db = TaskDb::open_from_service_spec()
        .await
        .map_err(RPCErrors::ReasonError)?;
    info!("task-manager database initialized");

    let handler = TaskManagerService::new(Arc::new(db));
    handler.start_stale_claim_sweeper();
    let server = TaskManagerHttpServer::new(handler);

    info!("start node task manager service...");
    const TASK_MANAGER_SERVICE_MAIN_PORT: u16 = 3380;
    let runner = Runner::new(TASK_MANAGER_SERVICE_MAIN_PORT);
    if let Err(err) = runner.add_http_server("/kapi/task-manager".to_string(), Arc::new(server)) {
        error!("failed to add task manager http server: {:?}", err);
    }
    if let Err(err) = runner.run().await {
        error!("task manager runner exited with error: {:?}", err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::RdbBackend;
    use serde_json::json;
    use std::net::IpAddr;
    use std::str::FromStr;
    use std::sync::Once;
    use tempfile::tempdir;

    static INIT_LOGGING: Once = Once::new();

    fn create_rpc_request(method: &str, params: Value) -> RPCRequest {
        RPCRequest {
            method: method.to_string(),
            params,
            seq: 1,
            token: Some("".to_string()),
            trace_id: Some("".to_string()),
        }
    }

    fn create_rpc_request_with_token(
        method: &str,
        params: Value,
        user_id: &str,
        app_id: &str,
    ) -> RPCRequest {
        RPCRequest {
            method: method.to_string(),
            params,
            seq: 1,
            token: Some(json!({ "sub": user_id, "appid": app_id }).to_string()),
            trace_id: Some("".to_string()),
        }
    }

    async fn setup_test_environment() -> (
        buckyos_api::TaskManagerServerHandler<TaskManagerService>,
        tempfile::TempDir,
    ) {
        INIT_LOGGING.call_once(|| {
            buckyos_kit::init_logging("test_task_manager", false);
        });
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());

        let db = TaskDb::open(&conn, RdbBackend::Sqlite, None).await.unwrap();
        let service = TaskManagerService::new(Arc::new(db));
        let server = buckyos_api::TaskManagerServerHandler::new(service);
        (server, temp_dir)
    }

    async fn setup_test_service() -> (TaskManagerService, tempfile::TempDir) {
        INIT_LOGGING.call_once(|| {
            buckyos_kit::init_logging("test_task_manager", false);
        });
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());

        let db = TaskDb::open(&conn, RdbBackend::Sqlite, None).await.unwrap();
        let service = TaskManagerService::new(Arc::new(db));
        (service, temp_dir)
    }

    fn task_with_permissions(read: TaskScope, write: TaskScope) -> Task {
        let mut task = new_task(
            "permission_task".to_string(),
            "test_type".to_string(),
            "runner-1".to_string(),
            "user1".to_string(),
            "app1".to_string(),
            "session1".to_string(),
            None,
            TaskPermissions { read, write },
            json!({"small": true}),
        );
        task.id = 42;
        task.root_id = "42".to_string();
        task
    }

    #[test]
    fn root_event_id_rejects_kevent_invalid_root_id() {
        assert_eq!(
            TaskManagerService::root_event_id("workflow-default"),
            Some("/task_mgr/workflow-default".to_string())
        );
        assert_eq!(TaskManagerService::root_event_id("workflow#default"), None);
        assert_eq!(TaskManagerService::root_event_id("workflow/default"), None);
    }

    #[test]
    fn runner_task_ready_event_id_uses_runner_inbox_path() {
        assert_eq!(
            TaskManagerService::runner_task_ready_event_id("node-1"),
            Some("/task_mgr/runner/node-1/task_ready".to_string())
        );
        assert_eq!(
            TaskManagerService::runner_task_ready_event_id("app.control_panel"),
            Some("/task_mgr/runner/app.control_panel/task_ready".to_string())
        );
        assert_eq!(TaskManagerService::runner_task_ready_event_id(""), None);
        assert_eq!(
            TaskManagerService::runner_task_ready_event_id("bad/runner"),
            None
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_permission_scope_checks() {
        let (service, _temp_dir) = setup_test_service().await;
        let same_app = RequestContext {
            user_id: "user1".to_string(),
            app_id: "app1".to_string(),
        };
        let same_user_other_app = RequestContext {
            user_id: "user1".to_string(),
            app_id: "app2".to_string(),
        };
        let system_app = RequestContext {
            user_id: "admin".to_string(),
            app_id: "system".to_string(),
        };
        let other_user = RequestContext {
            user_id: "user2".to_string(),
            app_id: "app1".to_string(),
        };

        let private_task = task_with_permissions(TaskScope::Private, TaskScope::Private);
        assert!(service.can_read_task(&same_app, &private_task));
        assert!(service.can_write_task(&same_app, &private_task));
        assert!(!service.can_read_task(&same_user_other_app, &private_task));
        assert!(!service.can_write_task(&same_user_other_app, &private_task));

        let user_task = task_with_permissions(TaskScope::User, TaskScope::User);
        assert!(service.can_read_task(&same_user_other_app, &user_task));
        assert!(service.can_write_task(&same_user_other_app, &user_task));
        assert!(!service.can_read_task(&other_user, &user_task));

        let system_task = task_with_permissions(TaskScope::System, TaskScope::System);
        assert!(service.can_read_task(&system_app, &system_task));
        assert!(service.can_write_task(&system_app, &system_task));
        assert!(!service.can_read_task(&same_app, &system_task));

        let empty_context = RequestContext::empty();
        assert!(service.can_read_task(&empty_context, &private_task));
        assert!(service.can_write_task(&empty_context, &private_task));
    }

    #[test]
    fn test_runner_management_permission_checks() {
        let empty_context = RequestContext::empty();
        assert!(!TaskManagerService::can_manage_runner(
            &empty_context,
            "node-a"
        ));

        let system_context = RequestContext {
            user_id: "admin".to_string(),
            app_id: "system".to_string(),
        };
        assert!(TaskManagerService::can_manage_runner(
            &system_context,
            "node-a"
        ));

        let node_daemon_context = RequestContext {
            user_id: "node-a".to_string(),
            app_id: NODE_DAEMON_RUNNER_APP_ID.to_string(),
        };
        assert!(TaskManagerService::can_manage_runner(
            &node_daemon_context,
            "node-a"
        ));
        assert!(!TaskManagerService::can_manage_runner(
            &node_daemon_context,
            "node-b"
        ));

        let app_runner_context = RequestContext {
            user_id: "user1".to_string(),
            app_id: "app.runner".to_string(),
        };
        assert!(TaskManagerService::can_manage_runner(
            &app_runner_context,
            "app.runner"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_event_payload_omits_large_task_data_and_rate_limits_low_priority_changes() {
        let (service, _temp_dir) = setup_test_service().await;
        let before = task_with_permissions(TaskScope::User, TaskScope::User);
        let mut after = before.clone();
        after.data = json!({"blob": "x".repeat(TASK_EVENT_DATA_INLINE_LIMIT_BYTES + 128)});
        after.progress = 20.0;

        let payload =
            service.build_event_payload(&before, &after, TaskChangeKind::Data, "update_task_data");
        assert_eq!(payload["data_omitted"], true);
        assert!(payload["data_size"].as_u64().unwrap() > TASK_EVENT_DATA_INLINE_LIMIT_BYTES as u64);
        assert!(payload.get("data").is_none());

        assert!(service.check_and_update_rate_limit(after.id, TaskChangeKind::Progress));
        assert!(!service.check_and_update_rate_limit(after.id, TaskChangeKind::Progress));
        assert!(service.check_and_update_rate_limit(after.id, TaskChangeKind::Status));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_and_get_task() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "test_task",
            "task_type": "test_type",
            "app_name": "test_app",
            "data": {"key": "value"}
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        if let RPCResult::Success(result) = create_resp.result {
            let task_id = result["task_id"].as_i64().unwrap();
            assert!(task_id > 0);

            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["name"], "test_task");
                assert_eq!(result["task"]["task_type"], "test_type");
                assert_eq!(result["task"]["app_id"], "test_app");
            } else {
                panic!("Failed to get task");
            }
        } else {
            panic!("Failed to create task");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_add_and_list_task_notes_does_not_change_task_data() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_req = create_rpc_request(
            "create_task",
            json!({
                "name": "note_task",
                "task_type": "test_type",
                "app_id": "task-center",
                "user_id": "user1",
                "data": {"request": {"payload": "original"}}
            }),
        );
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let add_note_req = create_rpc_request(
            "add_task_note",
            json!({
                "task_id": task_id,
                "note_type": "human",
                "content": "Prefer the previous successful approach.",
                "data": {"source": "review"},
                "source_user_id": "user1",
                "source_app_id": "task-center"
            }),
        );
        let add_note_resp = server.handle_rpc_call(add_note_req, ip).await.unwrap();
        if let RPCResult::Success(result) = add_note_resp.result {
            assert!(result["note_id"].as_i64().unwrap() > 0);
            assert_eq!(result["note"]["task_id"], task_id);
            assert_eq!(
                result["note"]["content"],
                "Prefer the previous successful approach."
            );
            assert_eq!(result["note"]["data"]["source"], "review");
        } else {
            panic!("Failed to add task note");
        }

        let list_notes_req = create_rpc_request(
            "list_task_notes",
            json!({
                "task_id": task_id,
                "source_user_id": "user1",
                "source_app_id": "task-center"
            }),
        );
        let list_notes_resp = server.handle_rpc_call(list_notes_req, ip).await.unwrap();
        if let RPCResult::Success(result) = list_notes_resp.result {
            let notes = result["notes"].as_array().unwrap();
            assert_eq!(notes.len(), 1);
            assert_eq!(notes[0]["note_type"], "human");
            assert_eq!(notes[0]["author_user_id"], "user1");
            assert_eq!(notes[0]["author_app_id"], "task-center");
        } else {
            panic!("Failed to list task notes");
        }

        let get_req = create_rpc_request("get_task", json!({ "id": task_id }));
        let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
        if let RPCResult::Success(result) = get_resp.result {
            assert_eq!(
                result["task"]["data"],
                json!({"request": {"payload": "original"}})
            );
            assert_eq!(result["task"]["status"], "Pending");
            assert_eq!(result["task"]["progress"], 0.0);
        } else {
            panic!("Failed to get task after adding note");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_task_ignores_data_rootid_for_record_root_id() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "grouped_task",
            "task_type": "test_type",
            "app_name": "test_app",
            "data": {
                "rootid": "session-alpha"
            }
        });
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            let task_id = result["task_id"]
                .as_i64()
                .expect("task_id should be present");
            let expected_root_id = task_id.to_string();
            assert_eq!(
                result["task"]["root_id"].as_str(),
                Some(expected_root_id.as_str())
            );
            task_id
        } else {
            panic!("Failed to create task");
        };

        let list_req = create_rpc_request("list_tasks", json!({ "root_id": task_id.to_string() }));
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        if let RPCResult::Success(result) = list_resp.result {
            let tasks = result["tasks"].as_array().expect("tasks should be array");
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0]["id"], task_id);
            let expected_root_id = task_id.to_string();
            assert_eq!(
                tasks[0]["root_id"].as_str(),
                Some(expected_root_id.as_str())
            );
        } else {
            panic!("Failed to list tasks by root_id");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_task_uses_request_root_id_field() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "grouped_task_req_root",
            "task_type": "test_type",
            "app_name": "test_app",
            "root_id": "session-beta"
        });
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            assert_eq!(result["task"]["root_id"], "session-beta");
            result["task_id"]
                .as_i64()
                .expect("task_id should be present")
        } else {
            panic!("Failed to create task");
        };

        let list_req = create_rpc_request("list_tasks", json!({ "root_id": "session-beta" }));
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        if let RPCResult::Success(result) = list_resp.result {
            let tasks = result["tasks"].as_array().expect("tasks should be array");
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0]["id"], task_id);
            assert_eq!(tasks[0]["root_id"], "session-beta");
        } else {
            panic!("Failed to list tasks by root_id");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_child_task_inherits_parent_root_id() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let parent_req = create_rpc_request(
            "create_task",
            json!({
                "name": "parent_task",
                "task_type": "workflow/run",
                "app_id": "workflow",
                "user_id": "user1",
                "root_id": "workflow-root-1"
            }),
        );
        let parent_resp = server.handle_rpc_call(parent_req, ip).await.unwrap();
        let parent_id = if let RPCResult::Success(result) = parent_resp.result {
            assert_eq!(result["task"]["root_id"], "workflow-root-1");
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create parent task");
        };

        let child_req = create_rpc_request(
            "create_task",
            json!({
                "name": "child_task",
                "task_type": "workflow/step",
                "app_id": "workflow",
                "user_id": "user1",
                "parent_id": parent_id,
                "root_id": "ignored-child-root"
            }),
        );
        let child_resp = server.handle_rpc_call(child_req, ip).await.unwrap();
        let child_id = if let RPCResult::Success(result) = child_resp.result {
            assert_eq!(result["task"]["parent_id"], parent_id);
            assert_eq!(result["task"]["root_id"], "workflow-root-1");
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create child task");
        };

        let list_req = create_rpc_request("list_tasks", json!({ "root_id": "workflow-root-1" }));
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        if let RPCResult::Success(result) = list_resp.result {
            let ids = result["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|task| task["id"].as_i64().unwrap())
                .collect::<Vec<_>>();
            assert!(ids.contains(&parent_id));
            assert!(ids.contains(&child_id));
            assert_eq!(ids.len(), 2);
        } else {
            panic!("Failed to list task tree by root_id");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_claim_task_claims_pending_runner_task_once() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_req = create_rpc_request(
            "create_task",
            json!({
                "name": "claimable_task",
                "task_type": "scheduler.dispatch_thunk",
                "runner": "node-a",
                "app_id": "scheduler",
                "user_id": "user1"
            }),
        );
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create claimable task");
        };

        let unauthenticated_claim_req =
            create_rpc_request("claim_task", json!({ "id": task_id, "runner": "node-b" }));
        let unauthenticated_claim = server.handle_rpc_call(unauthenticated_claim_req, ip).await;
        assert!(matches!(
            unauthenticated_claim,
            Err(RPCErrors::NoPermission(_))
        ));

        let wrong_runner_req = create_rpc_request_with_token(
            "claim_task",
            json!({ "id": task_id, "runner": "node-b" }),
            "kernel",
            "system",
        );
        let wrong_runner_resp = server.handle_rpc_call(wrong_runner_req, ip).await.unwrap();
        if let RPCResult::Success(result) = wrong_runner_resp.result {
            assert!(result["task"].is_null());
        } else {
            panic!("Wrong runner claim should return success with null task");
        }

        let unauthorized_runner_req = create_rpc_request_with_token(
            "claim_task",
            json!({ "id": task_id, "runner": "node-a" }),
            "node-b",
            NODE_DAEMON_RUNNER_APP_ID,
        );
        let unauthorized_runner = server.handle_rpc_call(unauthorized_runner_req, ip).await;
        assert!(matches!(
            unauthorized_runner,
            Err(RPCErrors::NoPermission(_))
        ));

        let claim_req = create_rpc_request_with_token(
            "claim_task",
            json!({ "id": task_id, "runner": "node-a" }),
            "node-a",
            NODE_DAEMON_RUNNER_APP_ID,
        );
        let claim_resp = server.handle_rpc_call(claim_req, ip).await.unwrap();
        let (claim_token, mut lease_until) = if let RPCResult::Success(result) = claim_resp.result {
            assert_eq!(result["task"]["id"], task_id);
            assert_eq!(result["task"]["runner"], "node-a");
            assert_eq!(result["task"]["status"], "Running");
            let claim_token = result["claim_token"]
                .as_str()
                .expect("claim_token should be returned")
                .to_string();
            let lease_until = result["lease_until"]
                .as_u64()
                .expect("lease_until should be returned");
            assert!(!claim_token.is_empty());
            assert!(lease_until > 0);
            (claim_token, lease_until)
        } else {
            panic!("Matching runner should claim task");
        };

        let second_claim_req = create_rpc_request_with_token(
            "claim_task",
            json!({ "id": task_id, "runner": "node-a" }),
            "node-a",
            NODE_DAEMON_RUNNER_APP_ID,
        );
        let second_claim_resp = server.handle_rpc_call(second_claim_req, ip).await.unwrap();
        if let RPCResult::Success(result) = second_claim_resp.result {
            assert!(result["task"].is_null());
        } else {
            panic!("Second claim should return success with null task");
        }

        let heartbeat_req = create_rpc_request_with_token(
            "heartbeat_task_claim",
            json!({
                "id": task_id,
                "claim_token": claim_token,
                "lease_ms": 120000
            }),
            "node-a",
            NODE_DAEMON_RUNNER_APP_ID,
        );
        let heartbeat_resp = server.handle_rpc_call(heartbeat_req, ip).await.unwrap();
        if let RPCResult::Success(result) = heartbeat_resp.result {
            assert!(result["lease_until"].as_u64().unwrap() >= lease_until);
            lease_until = result["lease_until"].as_u64().unwrap();
        } else {
            panic!("Heartbeat should renew the claim");
        }

        let unauthorized_all_requeue_req = create_rpc_request_with_token(
            "requeue_stale_task_claims",
            json!({ "now": lease_until + 1 }),
            "node-a",
            NODE_DAEMON_RUNNER_APP_ID,
        );
        let unauthorized_all_requeue = server
            .handle_rpc_call(unauthorized_all_requeue_req, ip)
            .await;
        assert!(matches!(
            unauthorized_all_requeue,
            Err(RPCErrors::NoPermission(_))
        ));

        let requeue_req = create_rpc_request_with_token(
            "requeue_stale_task_claims",
            json!({ "now": lease_until + 1, "runner": "node-a" }),
            "node-a",
            NODE_DAEMON_RUNNER_APP_ID,
        );
        let requeue_resp = server.handle_rpc_call(requeue_req, ip).await.unwrap();
        if let RPCResult::Success(result) = requeue_resp.result {
            assert_eq!(result["requeued_count"], 1);
        } else {
            panic!("Expired claim should be requeued");
        }

        let get_req = create_rpc_request("get_task", json!({ "id": task_id }));
        let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
        if let RPCResult::Success(result) = get_resp.result {
            assert_eq!(result["task"]["status"], "Pending");
        } else {
            panic!("Failed to read requeued task");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_reclaim_stale_task_claims_applies_policy() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let mut task_ids = Vec::new();
        let tasks = [
            ("retry_task", TASK_TYPE_SCHEDULER_DISPATCH_THUNK, json!({})),
            ("manual_task", TASK_TYPE_AGENT_DELEGATE, json!({})),
            (
                "fail_task",
                "custom.runner_task",
                json!({ "reclaim_policy": "fail" }),
            ),
        ];

        for (name, task_type, data) in tasks {
            let create_req = create_rpc_request(
                "create_task",
                json!({
                    "name": name,
                    "task_type": task_type,
                    "runner": "node-a",
                    "app_id": "scheduler",
                    "user_id": "user1",
                    "data": data
                }),
            );
            let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
            let task_id = if let RPCResult::Success(result) = create_resp.result {
                result["task_id"].as_i64().unwrap()
            } else {
                panic!("Failed to create reclaim policy task");
            };
            task_ids.push(task_id);
        }

        let mut max_lease_until = 0;
        for task_id in &task_ids {
            let claim_req = create_rpc_request_with_token(
                "claim_task",
                json!({
                    "id": task_id,
                    "runner": "node-a",
                    "lease_ms": 1000
                }),
                "kernel",
                "system",
            );
            let claim_resp = server.handle_rpc_call(claim_req, ip).await.unwrap();
            if let RPCResult::Success(result) = claim_resp.result {
                max_lease_until = max_lease_until.max(result["lease_until"].as_u64().unwrap());
                assert_eq!(result["task"]["status"], "Running");
            } else {
                panic!("Failed to claim reclaim policy task");
            }
        }

        let reclaim_req = create_rpc_request_with_token(
            "requeue_stale_task_claims",
            json!({ "now": max_lease_until + 1 }),
            "kernel",
            "system",
        );
        let reclaim_resp = server.handle_rpc_call(reclaim_req, ip).await.unwrap();
        if let RPCResult::Success(result) = reclaim_resp.result {
            assert_eq!(result["requeued_count"], 1);
            assert_eq!(result["failed_count"], 1);
            assert_eq!(result["manual_count"], 1);
        } else {
            panic!("Failed to reclaim stale claims");
        }

        let expected = [
            (task_ids[0], "Pending", None),
            (
                task_ids[1],
                "Running",
                Some(STALE_TASK_CLAIM_MANUAL_MESSAGE),
            ),
            (task_ids[2], "Failed", Some(STALE_TASK_CLAIM_FAILED_MESSAGE)),
        ];
        for (task_id, status, message) in expected {
            let get_req = create_rpc_request("get_task", json!({ "id": task_id }));
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["status"], status);
                if let Some(message) = message {
                    assert_eq!(result["task"]["message"], message);
                }
            } else {
                panic!("Failed to get reclaimed task");
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        for i in 1..4 {
            let create_params = json!({
                "name": format!("test_task_{}", i),
                "task_type": "test_type",
                "app_name": "test_app"
            });

            let create_req = create_rpc_request("create_task", create_params);
            let _ = server.handle_rpc_call(create_req, ip).await.unwrap();
        }

        let list_req = create_rpc_request("list_tasks", json!({}));
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();

        if let RPCResult::Success(result) = list_resp.result {
            let tasks = result["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 3);
        } else {
            panic!("Failed to list tasks");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks_by_app() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params1 = json!({
            "name": "app1_task",
            "task_type": "test_type",
            "app_name": "app1"
        });

        let create_params2 = json!({
            "name": "app2_task",
            "task_type": "test_type",
            "app_name": "app2"
        });

        let create_req1 = create_rpc_request("create_task", create_params1);
        let create_req2 = create_rpc_request("create_task", create_params2);
        let _ = server.handle_rpc_call(create_req1, ip).await.unwrap();
        let _ = server.handle_rpc_call(create_req2, ip).await.unwrap();

        let list_params = json!({
            "app_id": "app1"
        });

        let list_req = create_rpc_request("list_tasks", list_params);
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();

        if let RPCResult::Success(result) = list_resp.result {
            let tasks = result["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0]["app_id"], "app1");
        } else {
            panic!("Failed to list tasks by app");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_and_delete_tasks_by_session() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        for (name, app_id, session_id) in [
            ("session_task_1", "app1", "session-alpha"),
            ("session_task_2", "app2", "session-alpha"),
            ("session_task_3", "app1", "session-beta"),
        ] {
            let create_params = json!({
                "name": name,
                "task_type": "test_type",
                "app_id": app_id,
                "session_id": session_id
            });
            let create_req = create_rpc_request("create_task", create_params);
            let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
            if let RPCResult::Success(result) = create_resp.result {
                assert_eq!(result["task"]["session_id"], session_id);
                assert_eq!(result["task"]["app_id"], app_id);
            } else {
                panic!("Failed to create session task");
            }
        }

        let list_req = create_rpc_request("list_tasks", json!({ "session_id": "session-alpha" }));
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        if let RPCResult::Success(result) = list_resp.result {
            let tasks = result["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 2);
            assert!(tasks
                .iter()
                .all(|task| task["session_id"] == "session-alpha"));
        } else {
            panic!("Failed to list tasks by session");
        }

        let delete_req = create_rpc_request(
            "delete_tasks_by_session",
            json!({ "session_id": "session-alpha" }),
        );
        let delete_resp = server.handle_rpc_call(delete_req, ip).await.unwrap();
        if let RPCResult::Success(result) = delete_resp.result {
            assert_eq!(result["deleted_count"], 2);
        } else {
            panic!("Failed to delete tasks by session");
        }

        let list_req = create_rpc_request("list_tasks", json!({}));
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        if let RPCResult::Success(result) = list_resp.result {
            let tasks = result["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0]["session_id"], "session-beta");
        } else {
            panic!("Failed to list remaining tasks");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_status() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "status_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let update_params = json!({
            "id": task_id,
            "status": "Running"
        });

        let update_req = create_rpc_request("update_task_status", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();

        if let RPCResult::Success(_) = update_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["status"], "Running");
            } else {
                panic!("Failed to get task after status update");
            }
        } else {
            panic!("Failed to update task status");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_progress() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "progress_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let update_params = json!({
            "id": task_id,
            "completed_items": 5,
            "total_items": 10
        });

        let update_req = create_rpc_request("update_task_progress", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();

        if let RPCResult::Success(_) = update_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["progress"], 50.0);
                assert!(result["task"]["data"].as_object().unwrap().is_empty());
            } else {
                panic!("Failed to get task after progress update");
            }
        } else {
            panic!("Failed to update task progress");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_error() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "error_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let update_params = json!({
            "id": task_id,
            "error_message": "Test error occurred"
        });

        let update_req = create_rpc_request("update_task_error", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();

        if let RPCResult::Success(_) = update_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["message"], "Test error occurred");
                assert_eq!(result["task"]["status"], "Failed");
            } else {
                panic!("Failed to get task after error update");
            }
        } else {
            panic!("Failed to update task error");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_data() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "data_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let update_params = json!({
            "id": task_id,
            "data": {"updated": true, "value": "new data"}
        });

        let update_req = create_rpc_request("update_task_data", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();

        if let RPCResult::Success(_) = update_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["data"]["updated"], true);
                assert_eq!(result["task"]["data"]["value"], "new data");
            } else {
                panic!("Failed to get task after data update");
            }
        } else {
            panic!("Failed to update task data");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_delete_task() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "delete_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let delete_params = json!({
            "id": task_id
        });

        let delete_req = create_rpc_request("delete_task", delete_params);
        let delete_resp = server.handle_rpc_call(delete_req, ip).await.unwrap();

        if let RPCResult::Success(_) = delete_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_result = server.handle_rpc_call(get_req, ip).await;
            assert!(
                get_result.is_err(),
                "Unexpected success when getting deleted task"
            );
        } else {
            panic!("Failed to delete task");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_invalid_method() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let req = create_rpc_request("invalid_method", json!({}));
        let result = server.handle_rpc_call(req, ip).await;

        assert!(matches!(result, Err(RPCErrors::UnknownMethod(_))));
    }
}
