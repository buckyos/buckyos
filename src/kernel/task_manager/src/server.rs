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

// Keep the inlined data small enough that the full event (envelope + task
// fields + data) fits in a single shared-ringbuffer slot (SLOT_DATA_SIZE =
// 2048). Envelope + the 13 top-level task fields take ~400-700 bytes, so
// 1300 leaves headroom.
const TASK_EVENT_DATA_INLINE_LIMIT_BYTES: usize = 1300;
const TASK_EVENT_RATE_LIMIT: Duration = Duration::from_secs(1);

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

/// The caller identity resolved from the *verified* session token.
///
/// `zone_trusted` marks callers whose token was signed by the zone owner key
/// or a device key (kernel/frame services and the owner themselves). Those
/// callers form the zone's trusted computing base: they may create tasks on
/// behalf of an already-authenticated business user and may read/write any
/// task. Tokens issued by verify-hub (interactive user/app sessions) are
/// never zone-trusted and are restricted to their own tasks via TaskScope.
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub user_id: String,
    pub app_id: String,
    pub zone_trusted: bool,
    /// The verified token's `sudo` claim, preserved as-is. Only meaningful
    /// on verify-hub (interactive) tokens: an elevated session. TaskMgr
    /// authorization ignores it; the dispatcher's approval gate treats
    /// `zone_trusted || sudo` as the manual-release admin grade.
    pub sudo: bool,
}

/// Verifies the raw session token of a request. Production uses the runtime's
/// trust-key set; tests inject a verifier with a fixed key so the full
/// signature path is still exercised without a global runtime.
#[async_trait]
pub trait SessionTokenVerifier: Send + Sync {
    async fn verify(&self, token: &str) -> Result<RPCSessionToken>;
}

pub struct RuntimeSessionTokenVerifier;

#[async_trait]
impl SessionTokenVerifier for RuntimeSessionTokenVerifier {
    async fn verify(&self, token: &str) -> Result<RPCSessionToken> {
        get_buckyos_api_runtime()?
            .verify_trusted_session_token(token)
            .await
    }
}

#[derive(Clone)]
struct TaskManagerService {
    kevent_client: KEventClient,
    db: Arc<TaskDb>,
    token_verifier: Arc<dyn SessionTokenVerifier>,
    last_event_at: Arc<StdMutex<HashMap<i64, Instant>>>,
}

impl TaskManagerService {
    /// The KEvent client is injected rather than built here: which transport
    /// is correct depends on where the process runs, and only
    /// `BuckyOSRuntime::get_kevent_client` knows that.
    pub fn new(
        db: Arc<TaskDb>,
        kevent_client: KEventClient,
        token_verifier: Arc<dyn SessionTokenVerifier>,
    ) -> Self {
        TaskManagerService {
            kevent_client,
            db,
            token_verifier,
            last_event_at: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// Resolve the caller identity from the request's session token.
    /// Fail closed: no token / bad signature => NoPermission. Identity is
    /// never taken from the request payload.
    async fn authenticate(&self, ctx: &RPCContext) -> Result<RequestContext> {
        let token = ctx
            .token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                RPCErrors::NoPermission("task-manager requires a session token".to_string())
            })?;
        let verified = self.token_verifier.verify(token).await?;
        let (user_id, app_id) = verified.get_subs()?;
        if user_id.trim().is_empty() {
            return Err(RPCErrors::InvalidToken(
                "session token has an empty subject".to_string(),
            ));
        }
        // verify-hub issued tokens are interactive user/app sessions; every
        // other verifiable signer (owner key, device key) is zone-trusted.
        let zone_trusted = verified.iss.as_deref() != Some(VERIFY_HUB_UNIQUE_ID);
        Ok(RequestContext {
            user_id,
            app_id,
            zone_trusted,
            sudo: verified.sudo,
        })
    }

    /// Resolve the owner identity recorded on a new task. Zone-trusted
    /// callers may put an already-authenticated business identity into the
    /// request; everyone else must match their own token identity exactly.
    fn resolve_task_owner(
        request_ctx: &RequestContext,
        requested_user_id: &str,
        requested_app_id: &str,
    ) -> Result<(String, String)> {
        let requested_user_id = requested_user_id.trim();
        let requested_app_id = requested_app_id.trim();

        if request_ctx.zone_trusted {
            let user_id = if requested_user_id.is_empty() {
                request_ctx.user_id.clone()
            } else {
                requested_user_id.to_string()
            };
            let app_id = if requested_app_id.is_empty() {
                request_ctx.app_id.clone()
            } else {
                requested_app_id.to_string()
            };
            return Ok((user_id, app_id));
        }

        if !requested_user_id.is_empty() && requested_user_id != request_ctx.user_id {
            return Err(RPCErrors::NoPermission(format!(
                "caller {} cannot create tasks owned by {}",
                request_ctx.user_id, requested_user_id
            )));
        }
        if !requested_app_id.is_empty() && requested_app_id != request_ctx.app_id {
            return Err(RPCErrors::NoPermission(format!(
                "caller app {} cannot create tasks owned by app {}",
                request_ctx.app_id, requested_app_id
            )));
        }
        Ok((request_ctx.user_id.clone(), request_ctx.app_id.clone()))
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

    fn scope_allows(ctx: &RequestContext, task: &Task, scope: TaskScope) -> bool {
        // Zone-trusted callers (owner/device signed tokens) are the trusted
        // computing base and bypass scope checks; everyone else is bound to
        // the task owner identity recorded at create time.
        if ctx.zone_trusted {
            return true;
        }
        match scope {
            TaskScope::Private => task.user_id == ctx.user_id && task.app_id == ctx.app_id,
            TaskScope::User => task.user_id == ctx.user_id,
            TaskScope::System => false,
        }
    }

    fn can_read_task(&self, ctx: &RequestContext, task: &Task) -> bool {
        Self::scope_allows(ctx, task, task.permissions.read)
    }

    fn can_write_task(&self, ctx: &RequestContext, task: &Task) -> bool {
        Self::scope_allows(ctx, task, task.permissions.write)
    }

    async fn load_task(&self, id: i64) -> Result<Task> {
        let task = self
            .db
            .get_task(id)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        task.ok_or_else(|| RPCErrors::ReasonError(format!("Task {} not found", id)))
    }

    async fn list_download_tasks_for_owner(&self, user_id: &str, app_id: &str) -> Result<Vec<Task>> {
        let user_id = (!user_id.trim().is_empty()).then_some(user_id);
        let app_id = (!app_id.trim().is_empty()).then_some(app_id);
        self.db
            .list_tasks_filtered(
                app_id,
                None,
                Some(DOWNLOAD_TASK_TYPE),
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
        let request_ctx = self.authenticate(&ctx).await?;
        let (owner_user_id, owner_app_id) =
            Self::resolve_task_owner(&request_ctx, user_id, app_id)?;
        let permissions = opts.permissions.unwrap_or_default();
        let data = data.unwrap_or_else(|| json!({}));

        let mut task = new_task(
            name.to_string(),
            task_type.to_string(),
            owner_user_id,
            owner_app_id,
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

        let request_ctx = self.authenticate(&ctx).await?;
        let (owner_user_id, owner_app_id) =
            Self::resolve_task_owner(&request_ctx, user_id, app_id)?;
        let resolved_objid = objid.or_else(|| infer_objid_from_url(download_url));
        let scoped_tasks = self
            .list_download_tasks_for_owner(owner_user_id.as_str(), owner_app_id.as_str())
            .await?;

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

    async fn handle_get_task(&self, id: i64, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(id).await?;
        if !self.can_read_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to read task".to_string(),
            ));
        }

        Ok(task)
    }

    async fn handle_add_task_note(
        &self,
        task_id: i64,
        note_type: Option<&str>,
        content: &str,
        data: Option<Value>,
        ctx: RPCContext,
    ) -> Result<TaskNote> {
        let content = content.trim();
        if content.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "note content is required".to_string(),
            ));
        }

        let request_ctx = self.authenticate(&ctx).await?;
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

    async fn handle_list_task_notes(&self, task_id: i64, ctx: RPCContext) -> Result<Vec<TaskNote>> {
        let request_ctx = self.authenticate(&ctx).await?;
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

    async fn handle_list_tasks(&self, filter: TaskFilter, ctx: RPCContext) -> Result<Vec<Task>> {
        let request_ctx = self.authenticate(&ctx).await?;
        let tasks = self
            .db
            .list_tasks_filtered(
                filter.app_id.as_deref(),
                filter.session_id.as_deref(),
                filter.task_type.as_deref(),
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
        time_range: Range<u64>,
        ctx: RPCContext,
    ) -> Result<Vec<Task>> {
        let request_ctx = self.authenticate(&ctx).await?;
        let tasks = self
            .db
            .list_tasks_filtered(app_id, session_id, task_type, None, None, None, None)
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
        ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
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

    async fn handle_cancel_task(&self, id: i64, recursive: bool, ctx: RPCContext) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
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
                .list_tasks_filtered(None, None, None, None, None, Some(root_id.as_str()), None)
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

    async fn handle_get_subtasks(&self, parent_id: i64, ctx: RPCContext) -> Result<Vec<Task>> {
        let request_ctx = self.authenticate(&ctx).await?;
        let tasks = self
            .db
            .list_tasks_filtered(None, None, None, None, Some(parent_id), None, None)
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
        ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
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
        ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
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
        ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
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

    async fn handle_update_task_data(&self, id: i64, data: Value, ctx: RPCContext) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
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

    async fn handle_delete_task(&self, id: i64, ctx: RPCContext) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
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
        ctx: RPCContext,
    ) -> Result<u64> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "session_id is required".to_string(),
            ));
        }

        let request_ctx = self.authenticate(&ctx).await?;
        let tasks = self
            .db
            .list_tasks_filtered(None, Some(session_id), None, None, None, None, None)
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

    let kevent_client = get_buckyos_api_runtime()
        .map_err(|err| RPCErrors::ReasonError(format!("api runtime unavailable: {}", err)))?
        .get_kevent_client()
        .await?;
    let handler = TaskManagerService::new(
        Arc::new(db),
        kevent_client,
        Arc::new(RuntimeSessionTokenVerifier),
    );
    let server = TaskManagerHttpServer::new(handler);

    info!("start node task manager service...");
    const TASK_MANAGER_SERVICE_MAIN_PORT: u16 = 3380;
    let runner = Runner::new(TASK_MANAGER_SERVICE_MAIN_PORT);
    if let Err(err) = runner.add_http_server("/kapi/task-manager".to_string(), Arc::new(server)) {
        error!("failed to add task manager http server: {:?}", err);
    }

    // Task Dispatch Center: same process/port, second kapi path, independent
    // store and authorization. The task service must keep working when the
    // dispatcher fails to start (missing rdb instance config on older
    // zones), so this is strictly best-effort.
    match crate::dispatcher::start_task_dispatcher(Arc::new(RuntimeSessionTokenVerifier)).await {
        Ok(dispatcher_service) => {
            let dispatcher_server =
                crate::dispatcher::TaskDispatcherHttpServer::new(dispatcher_service);
            if let Err(err) = runner
                .add_http_server("/kapi/task-dispatcher".to_string(), Arc::new(dispatcher_server))
            {
                error!("failed to add task dispatcher http server: {:?}", err);
            } else {
                info!("task dispatch center mounted at /kapi/task-dispatcher");
            }
        }
        Err(err) => {
            warn!(
                "task dispatch center not started (task manager keeps running): {:?}",
                err
            );
        }
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
    use jsonwebtoken::{DecodingKey, EncodingKey};
    use serde_json::json;
    use std::collections::HashMap;
    use std::net::IpAddr;
    use std::str::FromStr;
    use std::sync::Once;
    use tempfile::tempdir;

    static INIT_LOGGING: Once = Once::new();

    // Fixed ed25519 test keypair (same material as the node_daemon tests).
    const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#;
    const TEST_PUBLIC_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

    const ZONE_USER: &str = "ood1";
    const ZONE_APP: &str = "task-manager-test";

    /// Verifies against the fixed test key: the real signature/exp path runs,
    /// only the trust-key lookup is replaced.
    struct StaticKeyVerifier {
        key: DecodingKey,
    }

    #[async_trait]
    impl SessionTokenVerifier for StaticKeyVerifier {
        async fn verify(&self, token: &str) -> Result<RPCSessionToken> {
            let mut parsed = RPCSessionToken::from_string(token)?;
            parsed.verify_by_key(&self.key)?;
            Ok(parsed)
        }
    }

    fn test_encoding_key() -> EncodingKey {
        EncodingKey::from_ed_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap()
    }

    /// Self-signed token (iss == sub): what kernel/frame services present.
    /// Treated as zone-trusted by the server.
    fn zone_token(user_id: &str, app_id: &str) -> String {
        let (jwt, _) =
            RPCSessionToken::generate_jwt_token(user_id, app_id, None, &test_encoding_key())
                .unwrap();
        jwt
    }

    /// verify-hub style token (iss == "verify-hub"): an interactive user/app
    /// session. Never zone-trusted.
    fn user_token(user_id: &str, app_id: &str) -> String {
        let now = buckyos_kit::buckyos_get_unix_timestamp();
        let session_token = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            exp: Some(now + 3600),
            iss: Some(VERIFY_HUB_UNIQUE_ID.to_string()),
            jti: None,
            sub: Some(user_id.to_string()),
            appid: Some(app_id.to_string()),
            sudo: false,
            extra: HashMap::new(),
        };
        session_token.generate_jwt(None, &test_encoding_key()).unwrap()
    }

    fn rpc_request_with_token(method: &str, params: Value, token: Option<String>) -> RPCRequest {
        RPCRequest {
            method: method.to_string(),
            params,
            seq: 1,
            token,
            trace_id: Some("".to_string()),
        }
    }

    /// Default request: a zone-trusted service caller.
    fn create_rpc_request(method: &str, params: Value) -> RPCRequest {
        rpc_request_with_token(method, params, Some(zone_token(ZONE_USER, ZONE_APP)))
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
        let verifier = StaticKeyVerifier {
            key: DecodingKey::from_ed_components(TEST_PUBLIC_X).unwrap(),
        };
        let service = TaskManagerService::new(
            Arc::new(db),
            KEventClient::new_local(TASK_MANAGER_SERVICE_NAME),
            Arc::new(verifier),
        );
        let server = buckyos_api::TaskManagerServerHandler::new(service);
        (server, temp_dir)
    }

    fn expect_success(resp: RPCResponse, context: &str) -> Value {
        match resp.result {
            RPCResult::Success(value) => value,
            RPCResult::Failed(err) => panic!("{} failed: {}", context, err),
        }
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

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_and_get_task() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "test_task",
            "task_type": "test_type",
            "app_id": "test_app",
            "data": {"key": "value"}
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let result = expect_success(create_resp, "create_task");
        let task_id = result["task_id"].as_i64().unwrap();
        assert!(task_id > 0);
        // Owner user comes from the (zone-trusted) token subject; app_id was
        // delegated in the payload.
        assert_eq!(result["task"]["user_id"], ZONE_USER);
        assert_eq!(result["task"]["app_id"], "test_app");
        // runner is gone from the task wire format.
        assert!(result["task"].get("runner").is_none());

        let get_req = create_rpc_request("get_task", json!({ "id": task_id }));
        let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

        let result = expect_success(get_resp, "get_task");
        assert_eq!(result["task"]["name"], "test_task");
        assert_eq!(result["task"]["task_type"], "test_type");
        assert_eq!(result["task"]["app_id"], "test_app");
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
        let task_id = expect_success(create_resp, "create_task")["task_id"]
            .as_i64()
            .unwrap();

        let add_note_req = create_rpc_request(
            "add_task_note",
            json!({
                "task_id": task_id,
                "note_type": "human",
                "content": "Prefer the previous successful approach.",
                "data": {"source": "review"}
            }),
        );
        let add_note_resp = server.handle_rpc_call(add_note_req, ip).await.unwrap();
        let result = expect_success(add_note_resp, "add_task_note");
        assert!(result["note_id"].as_i64().unwrap() > 0);
        assert_eq!(result["note"]["task_id"], task_id);
        assert_eq!(
            result["note"]["content"],
            "Prefer the previous successful approach."
        );
        assert_eq!(result["note"]["data"]["source"], "review");

        let list_notes_req = create_rpc_request("list_task_notes", json!({ "task_id": task_id }));
        let list_notes_resp = server.handle_rpc_call(list_notes_req, ip).await.unwrap();
        let result = expect_success(list_notes_resp, "list_task_notes");
        let notes = result["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0]["note_type"], "human");
        // Note authorship comes from the verified token, not the payload.
        assert_eq!(notes[0]["author_user_id"], ZONE_USER);
        assert_eq!(notes[0]["author_app_id"], ZONE_APP);

        let get_req = create_rpc_request("get_task", json!({ "id": task_id }));
        let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
        let result = expect_success(get_resp, "get_task");
        assert_eq!(
            result["task"]["data"],
            json!({"request": {"payload": "original"}})
        );
        assert_eq!(result["task"]["status"], "Pending");
        assert_eq!(result["task"]["progress"], 0.0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_requests_without_token_are_rejected() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        for (method, params) in [
            ("create_task", json!({"name": "t", "task_type": "x"})),
            ("get_task", json!({"id": 1})),
            ("list_tasks", json!({})),
            ("update_task_status", json!({"id": 1, "status": "Running"})),
        ] {
            for token in [None, Some("".to_string())] {
                let req = rpc_request_with_token(method, params.clone(), token);
                let result = server.handle_rpc_call(req, ip).await;
                assert!(
                    result.is_err(),
                    "{} should fail without a session token",
                    method
                );
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_user_token_cannot_forge_task_owner() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        // A normal (verify-hub issued) app session tries to create a task
        // owned by another user / app: rejected.
        for params in [
            json!({"name": "t", "task_type": "app.install", "user_id": "root"}),
            json!({"name": "t", "task_type": "app.install", "app_id": "control-panel"}),
        ] {
            let req = rpc_request_with_token(
                "create_task",
                params,
                Some(user_token("alice", "some-app")),
            );
            let result = server.handle_rpc_call(req, ip).await;
            assert!(result.is_err(), "forged owner should be rejected");
        }

        // Without forged fields the task is recorded under the token identity.
        let req = rpc_request_with_token(
            "create_task",
            json!({"name": "t", "task_type": "test_type"}),
            Some(user_token("alice", "some-app")),
        );
        let resp = server.handle_rpc_call(req, ip).await.unwrap();
        let result = expect_success(resp, "create_task with own identity");
        assert_eq!(result["task"]["user_id"], "alice");
        assert_eq!(result["task"]["app_id"], "some-app");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_zone_trusted_caller_can_create_on_behalf_of_user() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        // control-panel style flow: service token, business user in payload.
        let req = rpc_request_with_token(
            "create_task",
            json!({
                "name": "install foo",
                "task_type": "app.install",
                "user_id": "alice",
                "app_id": "control-panel"
            }),
            Some(zone_token("ood1", "control-panel")),
        );
        let resp = server.handle_rpc_call(req, ip).await.unwrap();
        let result = expect_success(resp, "create_task on behalf of user");
        assert_eq!(result["task"]["user_id"], "alice");
        assert_eq!(result["task"]["app_id"], "control-panel");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_user_scope_blocks_other_users_and_allows_owner() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        // alice creates her own task (default read scope: User).
        let req = rpc_request_with_token(
            "create_task",
            json!({"name": "alice_task", "task_type": "test_type"}),
            Some(user_token("alice", "some-app")),
        );
        let resp = server.handle_rpc_call(req, ip).await.unwrap();
        let task_id = expect_success(resp, "create alice task")["task_id"]
            .as_i64()
            .unwrap();

        // bob cannot read or cancel it.
        let get_req = rpc_request_with_token(
            "get_task",
            json!({"id": task_id}),
            Some(user_token("bob", "some-app")),
        );
        assert!(server.handle_rpc_call(get_req, ip).await.is_err());

        let cancel_req = rpc_request_with_token(
            "cancel_task",
            json!({"id": task_id, "recursive": false}),
            Some(user_token("bob", "some-app")),
        );
        assert!(server.handle_rpc_call(cancel_req, ip).await.is_err());

        // bob does not see it in list_tasks either.
        let list_req = rpc_request_with_token(
            "list_tasks",
            json!({}),
            Some(user_token("bob", "some-app")),
        );
        let resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        let result = expect_success(resp, "bob list_tasks");
        assert_eq!(result["tasks"].as_array().unwrap().len(), 0);

        // alice reads her own task; default write scope is Private, so the
        // same user+app combination may update it.
        let get_req = rpc_request_with_token(
            "get_task",
            json!({"id": task_id}),
            Some(user_token("alice", "some-app")),
        );
        let resp = server.handle_rpc_call(get_req, ip).await.unwrap();
        let result = expect_success(resp, "alice get_task");
        assert_eq!(result["task"]["name"], "alice_task");

        // Zone-trusted service can read it too.
        let get_req = create_rpc_request("get_task", json!({"id": task_id}));
        let resp = server.handle_rpc_call(get_req, ip).await.unwrap();
        expect_success(resp, "zone get_task");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_task_with_runner_payload_gets_no_dispatch_semantics() {
        // Regression for the beta2.2 boundary: a well-formed `app.install`
        // task created directly through TaskMgr — even with the legacy
        // `runner` field in the payload — is stored as plain data. There is
        // no runner column, no runner filter and no task_ready inbox event,
        // so no runner-based executor can be tricked into picking it up.
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let req = rpc_request_with_token(
            "create_task",
            json!({
                "name": "install evil",
                "task_type": "app.install",
                "runner": "app.control_panel",
                "data": {"schema_version": 2, "request": {"source": {"kind": "pkg", "value": "evil"}, "user_id": "root"}}
            }),
            Some(user_token("mallory", "evil-app")),
        );
        let resp = server.handle_rpc_call(req, ip).await.unwrap();
        let result = expect_success(resp, "create app.install task as data");
        // The task exists, but only as mallory's own record.
        assert_eq!(result["task"]["user_id"], "mallory");
        assert_eq!(result["task"]["app_id"], "evil-app");
        assert!(result["task"].get("runner").is_none());

        // The legacy runner list filter is no longer part of the protocol:
        // it is ignored, and the query degrades to mallory's own tasks.
        let list_req = rpc_request_with_token(
            "list_tasks",
            json!({"runner": "app.control_panel", "status": "Pending"}),
            Some(user_token("mallory", "evil-app")),
        );
        let resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        let result = expect_success(resp, "list_tasks with legacy runner filter");
        let tasks = result["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["user_id"], "mallory");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_task_ignores_data_rootid_for_record_root_id() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "grouped_task",
            "task_type": "test_type",
            "app_id": "test_app",
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
            "app_id": "test_app",
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
    async fn test_list_tasks() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        for i in 1..4 {
            let create_params = json!({
                "name": format!("test_task_{}", i),
                "task_type": "test_type",
                "app_id": "test_app"
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
            "app_id": "app1"
        });

        let create_params2 = json!({
            "name": "app2_task",
            "task_type": "test_type",
            "app_id": "app2"
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
            "app_id": "test_app"
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
            "app_id": "test_app"
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
            "app_id": "test_app"
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
            "app_id": "test_app"
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
            "app_id": "test_app"
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
