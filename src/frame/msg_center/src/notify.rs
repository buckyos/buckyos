//! System Notify (`notify.*`, `doc/message_hub/System Notify.md`).
//!
//! A notification is a `kind = notify` message. Its home record is written
//! straight into the user's INBOX, in the owner session of its category
//! (`sysnotify:<category>`), by the system notify sender (the zone DID):
//! the user never added that sender as a contact, so contact grading would
//! park it in the request box. A notification of an Agent is also posted by
//! the Agent into the user's conversations with it, by level, and every such
//! copy leaves an observation record in the Agent's own INBOX session of that
//! conversation (`buckyos.notify.delivered`).
//!
//! The `notifies` row keeps what msg-center decided (ids, planned copies,
//! records written). The state of a `notice` is the state of its records,
//! the state of an `input` is its TaskMgr task. `send` is resumable: every
//! step is idempotent and the row remembers how far it got; the sweep
//! finishes rows a crash left behind.

use crate::contact_mgr::ContactMgr;
use crate::msg_box_db::{to_sql_i64, IdempotencyCommitOutcome, MsgBoxDbMgr};
use crate::msg_center::MessageCenter;
use crate::owner_session::is_in_process_call;
use async_trait::async_trait;
use buckyos_api::*;
use kRPC::{RPCContext, RPCErrors};
use log::{info, warn};
use name_lib::DID;
use ndn_lib::{MsgContent, MsgContentFormat, MsgObjKind, MsgObject, NamedObject, ObjId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::{Arc, RwLock};

const NOTIFY_SWEEP_INTERVAL_SECS: u64 = 30;
const IDEMPOTENCY_SCOPE_NOTIFY: &str = "notify";
const NOTIFY_SESSION_ORIGIN: &str = "sysnotify";
/// Latest records of a conversation looked at to find the user's last
/// message in it.
const CONVERSATION_SCAN_RECORDS: usize = 30;
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 200;
const MAX_TITLE_CHARS: usize = 200;

type Result<T> = std::result::Result<T, RPCErrors>;

fn denied(reason: impl Into<String>) -> RPCErrors {
    RPCErrors::NoPermission(reason.into())
}

fn db_error(e: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(format!("notify-db: {e}"))
}

fn category_title(category: NotifyCategory) -> &'static str {
    match category {
        NotifyCategory::Todo => "待办",
        NotifyCategory::Agent => "Agent",
        NotifyCategory::App => "应用通知",
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ----------------------------------------------------------------------
// TaskMgr
// ----------------------------------------------------------------------

/// What msg-center needs from TaskMgr for `input` notifications.
#[async_trait]
pub trait NotifyTasks: Send + Sync {
    /// Idempotent for the same `idempotency_key` and input.
    async fn create(&self, req: CreateTaskReq) -> Result<Task>;
    async fn grant(&self, task_id: &str, grants: Vec<TaskAclGrantSpec>) -> Result<()>;
    async fn get(&self, task_id: &str) -> Result<Task>;
    /// Close the task as canceled; a task already closed is no error.
    async fn cancel(&self, task_id: &str, request_id: &str) -> Result<()>;
}

/// TaskMgr of the zone; a fresh client for every call (the token renews).
pub struct ZoneNotifyTasks;

impl ZoneNotifyTasks {
    async fn client() -> Result<TaskManagerClient> {
        get_buckyos_api_runtime()?.get_task_mgr_client().await
    }
}

#[async_trait]
impl NotifyTasks for ZoneNotifyTasks {
    async fn create(&self, req: CreateTaskReq) -> Result<Task> {
        Self::client().await?.create_task(req).await
    }

    async fn grant(&self, task_id: &str, grants: Vec<TaskAclGrantSpec>) -> Result<()> {
        let client = Self::client().await?;
        let mut task = client.get_task(task_id).await?;
        for grant in grants {
            task = client
                .grant_task_access(GrantTaskAccessReq {
                    task_id: task_id.to_string(),
                    grant,
                    expected_revision: task.revision,
                })
                .await?;
        }
        Ok(())
    }

    async fn get(&self, task_id: &str) -> Result<Task> {
        Self::client().await?.get_task(task_id).await
    }

    async fn cancel(&self, task_id: &str, request_id: &str) -> Result<()> {
        let result = Self::client()
            .await?
            .request_control(RequestControlReq {
                task_id: task_id.to_string(),
                action: TaskControlAction::Cancel,
                request_id: request_id.to_string(),
                recursive: false,
                expected_revision: None,
            })
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(e) if task_mgr_error_code(&e) == Some(TASK_ERR_ALREADY_COMPLETED) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// Holder so `MessageCenter` stays `Clone + Debug` with a swappable backend.
pub struct NotifyTasksSlot(RwLock<Arc<dyn NotifyTasks>>);

impl NotifyTasksSlot {
    pub(crate) fn get(&self) -> Arc<dyn NotifyTasks> {
        self.0.read().unwrap().clone()
    }

    #[allow(dead_code)]
    pub(crate) fn set(&self, tasks: Arc<dyn NotifyTasks>) {
        *self.0.write().unwrap() = tasks;
    }
}

impl Default for NotifyTasksSlot {
    fn default() -> Self {
        Self(RwLock::new(Arc::new(ZoneNotifyTasks)))
    }
}

impl std::fmt::Debug for NotifyTasksSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NotifyTasksSlot")
    }
}

// ----------------------------------------------------------------------
// Rows
// ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotifyRowState {
    Delivering,
    Delivered,
    Closed,
}

impl NotifyRowState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Delivering => "delivering",
            Self::Delivered => "delivered",
            Self::Closed => "closed",
        }
    }
}

/// A conversation of the user with an Agent, from the Agent's INBOX.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Conversation {
    /// The Agent INBOX session.
    pub session_id: String,
    pub platform: String,
    /// The user's endpoint the Agent replies to.
    pub to: String,
    #[serde(default)]
    pub to_session: Option<String>,
    pub last_active_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NotifyRow {
    pub notify_id: String,
    pub to_user: String,
    pub owner_did: DID,
    pub requester: String,
    pub idempotency_key: String,
    /// `(user_id, app)` TaskMgr knows the requester as.
    #[serde(default)]
    pub task_principal: Option<(String, String)>,
    pub kind: NotifyKind,
    pub category: NotifyCategory,
    pub level: NotifyLevel,
    pub title: String,
    pub prompt: HumanTaskPrompt,
    pub source: NotifySource,
    pub sender: DID,
    pub url: String,
    pub created_at_ms: u64,
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
    pub state: NotifyRowState,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub task_granted: bool,
    /// Copies decided at send time (Agent sources only).
    #[serde(default)]
    pub planned: Option<Vec<Conversation>>,
    #[serde(default)]
    pub home: Option<NotifyRecordRef>,
    /// The home record was written unread (it counts for `read`).
    #[serde(default)]
    pub home_unread: bool,
    #[serde(default)]
    pub deliveries: Vec<NotifyDelivery>,
    /// Conversations whose observation record is written.
    #[serde(default)]
    pub observed: Vec<String>,
    #[serde(default)]
    pub acked: bool,
    #[serde(default)]
    pub acked_by: Option<String>,
    /// Final status of a `notice`; an `input` takes it from its task.
    #[serde(default)]
    pub closed_status: Option<NotifyStatus>,
    #[serde(default)]
    pub closed_reason: Option<String>,
    #[serde(default)]
    pub closed_at_ms: Option<u64>,
}

/// The `notifies` table.
#[derive(Clone, Debug)]
pub struct NotifyStore {
    db: MsgBoxDbMgr,
}

impl NotifyStore {
    pub async fn open(db: MsgBoxDbMgr) -> Result<Self> {
        for ddl in [
            "CREATE TABLE IF NOT EXISTS notifies (notify_id TEXT PRIMARY KEY, owner_did TEXT NOT NULL, requester TEXT NOT NULL, idempotency_key TEXT NOT NULL, state TEXT NOT NULL, created_at_ms BIGINT NOT NULL, row_json TEXT NOT NULL)",
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_notifies_requester_key ON notifies(requester, idempotency_key)",
            "CREATE INDEX IF NOT EXISTS idx_notifies_owner_created ON notifies(owner_did, created_at_ms)",
            "CREATE INDEX IF NOT EXISTS idx_notifies_state ON notifies(state)",
        ] {
            sqlx::query(ddl).execute(db.pool()).await.map_err(db_error)?;
        }
        Ok(Self { db })
    }

    fn decode(row: &sqlx::any::AnyRow) -> Result<NotifyRow> {
        let body: String = row.try_get("row_json").map_err(db_error)?;
        serde_json::from_str(&body).map_err(db_error)
    }

    /// Insert unless the requester already used the key; the stored row wins.
    async fn insert(&self, row: &NotifyRow) -> Result<NotifyRow> {
        let sql = self.db.render_sql(
            "INSERT INTO notifies (notify_id, owner_did, requester, idempotency_key, state, created_at_ms, row_json) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        );
        sqlx::query(&sql)
            .bind(row.notify_id.clone())
            .bind(row.owner_did.to_string())
            .bind(row.requester.clone())
            .bind(row.idempotency_key.clone())
            .bind(row.state.as_str().to_string())
            .bind(to_sql_i64(row.created_at_ms))
            .bind(serde_json::to_string(row).map_err(db_error)?)
            .execute(self.db.pool())
            .await
            .map_err(db_error)?;
        self.by_key(&row.requester, &row.idempotency_key)
            .await?
            .ok_or_else(|| db_error("notification row vanished after insert"))
    }

    pub(crate) async fn update(&self, row: &NotifyRow) -> Result<()> {
        let sql = self
            .db
            .render_sql("UPDATE notifies SET state = ?, row_json = ? WHERE notify_id = ?");
        sqlx::query(&sql)
            .bind(row.state.as_str().to_string())
            .bind(serde_json::to_string(row).map_err(db_error)?)
            .bind(row.notify_id.clone())
            .execute(self.db.pool())
            .await
            .map_err(db_error)?;
        Ok(())
    }

    pub(crate) async fn get(&self, notify_id: &str) -> Result<Option<NotifyRow>> {
        let sql = self
            .db
            .render_sql("SELECT row_json FROM notifies WHERE notify_id = ?");
        sqlx::query(&sql)
            .bind(notify_id.to_string())
            .fetch_optional(self.db.pool())
            .await
            .map_err(db_error)?
            .map(|r| Self::decode(&r))
            .transpose()
    }

    async fn by_key(&self, requester: &str, key: &str) -> Result<Option<NotifyRow>> {
        let sql = self.db.render_sql(
            "SELECT row_json FROM notifies WHERE requester = ? AND idempotency_key = ?",
        );
        sqlx::query(&sql)
            .bind(requester.to_string())
            .bind(key.to_string())
            .fetch_optional(self.db.pool())
            .await
            .map_err(db_error)?
            .map(|r| Self::decode(&r))
            .transpose()
    }

    async fn unclosed(&self) -> Result<Vec<NotifyRow>> {
        let sql = self
            .db
            .render_sql("SELECT row_json FROM notifies WHERE state <> 'closed'");
        sqlx::query(&sql)
            .fetch_all(self.db.pool())
            .await
            .map_err(db_error)?
            .iter()
            .map(Self::decode)
            .collect()
    }

    /// Newest first, after `cursor = (created_at_ms, notify_id)`.
    async fn page(
        &self,
        owner: Option<&DID>,
        cursor: Option<(u64, String)>,
        limit: usize,
    ) -> Result<Vec<NotifyRow>> {
        let mut sql = String::from("SELECT row_json FROM notifies WHERE 1 = 1");
        if owner.is_some() {
            sql.push_str(" AND owner_did = ?");
        }
        if cursor.is_some() {
            sql.push_str(" AND (created_at_ms < ? OR (created_at_ms = ? AND notify_id < ?))");
        }
        sql.push_str(" ORDER BY created_at_ms DESC, notify_id DESC LIMIT ?");
        let sql = self.db.render_sql(&sql);
        let mut query = sqlx::query(&sql);
        if let Some(owner) = owner {
            query = query.bind(owner.to_string());
        }
        if let Some((at, id)) = cursor {
            query = query.bind(to_sql_i64(at)).bind(to_sql_i64(at)).bind(id);
        }
        query
            .bind(limit as i64)
            .fetch_all(self.db.pool())
            .await
            .map_err(db_error)?
            .iter()
            .map(Self::decode)
            .collect()
    }
}

// ----------------------------------------------------------------------
// Caller
// ----------------------------------------------------------------------

/// Who calls `notify.*`, from the verified session token.
#[derive(Debug, Clone, Default)]
pub(crate) struct NotifyCaller {
    /// System services (and in-process calls) may notify anyone and name
    /// any category.
    pub system: bool,
    /// Idempotency scope and "who sent it": `<sub>|<target>`.
    pub requester: String,
    pub user_id: Option<String>,
    pub user_did: Option<DID>,
    /// Bare app id, or `system:<service>`.
    pub app_id: Option<String>,
    pub agent_did: Option<DID>,
    pub task_principal: Option<(String, String)>,
}

impl NotifyCaller {
    fn is_user(&self, row: &NotifyRow) -> bool {
        self.user_id.as_deref() == Some(row.to_user.as_str())
            || self.user_did.as_ref() == Some(&row.owner_did)
    }

    fn may_read(&self, row: &NotifyRow) -> bool {
        self.system || self.requester == row.requester || self.is_user(row)
    }

    fn may_act_for_user(&self, row: &NotifyRow) -> bool {
        self.system || self.is_user(row)
    }

    fn may_cancel(&self, row: &NotifyRow) -> bool {
        self.system || self.requester == row.requester
    }
}

impl MessageCenter {
    /// The sender of home records and the zone host of notification links.
    pub fn set_notify_identity(&self, sender: DID, zone_host: Option<String>) {
        *self.notify_identity.write().unwrap() = Some((sender, zone_host));
    }

    fn notify_identity(&self) -> (DID, Option<String>) {
        if let Some(identity) = self.notify_identity.read().unwrap().clone() {
            return identity;
        }
        match get_buckyos_api_runtime() {
            Ok(runtime) if runtime.zone_id.is_valid() => (
                runtime.zone_id.clone(),
                Some(runtime.zone_id.to_host_name()),
            ),
            _ => (DID::new("web", "localhost"), None),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn set_notify_tasks(&self, tasks: Arc<dyn NotifyTasks>) {
        self.notify_tasks.set(tasks);
    }

    /// The Agent an App is, when the App is the constructed App of a zone
    /// Agent of `owner` (AppId = AgentId).
    pub(crate) async fn agent_of_app(&self, app_id: &str, owner: Option<&DID>) -> Option<DID> {
        let did = AgentId::parse(app_id).ok()?.agent_did();
        let agent_owner = self.token_verifier.get().agent_owner(&did).await.ok()??;
        (owner.is_none() || owner == Some(&agent_owner)).then_some(did)
    }

    pub(crate) async fn notify_caller(&self, ctx: &RPCContext) -> Result<NotifyCaller> {
        if is_in_process_call(ctx) {
            return Ok(NotifyCaller {
                system: true,
                requester: "in-process".to_string(),
                ..Default::default()
            });
        }
        let token = ctx
            .token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| denied("session token required"))?;
        let verified = self
            .token_verifier
            .get()
            .verify(token)
            .await
            .map_err(|e| denied(format!("invalid session token: {e}")))?;
        let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session)
            .map_err(|e| denied(format!("invalid session claims: {e}")))?;
        let sub = verified
            .sub
            .clone()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| denied("session token has no subject"))?;
        let canonical = claims.target.canonical_key();
        let mut caller = NotifyCaller {
            requester: format!("{sub}|{canonical}"),
            task_principal: Some((sub.clone(), canonical.clone())),
            ..Default::default()
        };
        match claims.principal_kind {
            TokenPrincipalKind::System | TokenPrincipalKind::Device => {
                caller.system = true;
                caller.app_id = Some(canonical);
            }
            TokenPrincipalKind::User if buckyos_api::token_is_external(&verified) => {
                return Err(denied("External users have no System Notify access"));
            }
            TokenPrincipalKind::App | TokenPrincipalKind::User => {
                let user_did = self
                    .token_verifier
                    .get()
                    .resolve_user_did(&sub)
                    .await
                    .map_err(|e| denied(format!("cannot resolve user {sub}: {e}")))?;
                let app_id = claims.target.appid_claim().to_string();
                if claims.principal_kind == TokenPrincipalKind::App {
                    caller.agent_did = self.agent_of_app(&app_id, Some(&user_did)).await;
                }
                caller.app_id = Some(app_id);
                caller.user_id = Some(sub);
                caller.user_did = Some(user_did);
            }
            TokenPrincipalKind::Agent => {
                let did = DID::from_str(&sub)
                    .map_err(|e| denied(format!("agent token subject is no DID: {e}")))?;
                let owner = self
                    .token_verifier
                    .get()
                    .agent_owner(&did)
                    .await?
                    .ok_or_else(|| denied(format!("{sub} is no agent of this zone")))?;
                caller.app_id = AgentId::from_agent_did(&did)
                    .ok()
                    .map(|a| a.as_str().to_string());
                caller.agent_did = Some(did);
                caller.user_did = Some(owner);
            }
        }
        Ok(caller)
    }

    // ------------------------------------------------------------------
    // kRPC
    // ------------------------------------------------------------------

    pub(crate) async fn notify_rpc(
        &self,
        method: &str,
        params: Value,
        ctx: RPCContext,
    ) -> Result<Value> {
        fn parse<T: DeserializeOwned>(params: Value) -> Result<T> {
            serde_json::from_value(params).map_err(|e| RPCErrors::ParseRequestError(e.to_string()))
        }
        let action = match method {
            METHOD_NOTIFY_GET | METHOD_NOTIFY_LIST => "read",
            METHOD_NOTIFY_SEND | METHOD_NOTIFY_ACK | METHOD_NOTIFY_DISMISS
            | METHOD_NOTIFY_CANCEL => "write",
            _ => return Err(RPCErrors::UnknownMethod(method.to_string())),
        };
        self.authorize_resource(&ctx, NOTIFY_RBAC_RESOURCE, action)
            .await?;
        let caller = self.notify_caller(&ctx).await?;
        let result = match method {
            METHOD_NOTIFY_SEND => json!(self.notify_send(&caller, parse(params)?).await?),
            METHOD_NOTIFY_GET => {
                let req: NotifyGetReq = parse(params)?;
                json!(self.notify_get(&caller, &req.notify_id).await?)
            }
            METHOD_NOTIFY_LIST => json!(self.notify_list(&caller, parse(params)?).await?),
            METHOD_NOTIFY_ACK => {
                let req: NotifyAckReq = parse(params)?;
                json!(self.notify_ack(&caller, &req.notify_id, req.on_behalf).await?)
            }
            METHOD_NOTIFY_DISMISS => {
                let req: NotifyDismissReq = parse(params)?;
                json!(self.notify_dismiss(&caller, &req.notify_id).await?)
            }
            _ => {
                let req: NotifyCancelReq = parse(params)?;
                json!(
                    self.notify_cancel(&caller, &req.notify_id, req.reason)
                        .await?
                )
            }
        };
        Ok(result)
    }

    // ------------------------------------------------------------------
    // send
    // ------------------------------------------------------------------

    pub(crate) async fn notify_send(
        &self,
        caller: &NotifyCaller,
        req: NotifySendReq,
    ) -> Result<NotifyView> {
        let to_user = req.to_user.trim().to_string();
        if to_user.is_empty() {
            return Err(RPCErrors::ParseRequestError("to_user is required".into()));
        }
        let key = req.idempotency_key.trim().to_string();
        if key.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "idempotency_key is required".into(),
            ));
        }
        let title = req
            .prompt
            .title
            .as_deref()
            .or(req.prompt.message.as_deref())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| truncate(t, MAX_TITLE_CHARS))
            .ok_or_else(|| RPCErrors::ParseRequestError("prompt needs a title".into()))?;
        let owner_did = self
            .token_verifier
            .get()
            .resolve_user_did(&to_user)
            .await
            .map_err(|e| RPCErrors::ParseRequestError(format!("unknown user {to_user}: {e}")))?;
        if !caller.system
            && caller.user_id.as_deref() != Some(to_user.as_str())
            && caller.user_did.as_ref() != Some(&owner_did)
        {
            return Err(denied(format!(
                "{} may only notify its own user",
                caller.requester
            )));
        }
        let derived = if caller.agent_did.is_some() {
            NotifyCategory::Agent
        } else {
            NotifyCategory::App
        };
        let category = match req.category {
            Some(named) if caller.system => named,
            Some(named) if named != derived => {
                return Err(denied(format!(
                    "category {} is not for {}",
                    named.as_str(),
                    caller.requester
                )))
            }
            _ => derived,
        };
        let mut source = req.source.unwrap_or_default();
        source.app_id = caller.app_id.clone();
        source.agent_did = caller.agent_did.as_ref().map(|d| d.to_string());
        let mut prompt = req.prompt;
        prompt.title = Some(title.clone());
        if req.kind == NotifyKind::Notice {
            prompt.controls.clear();
        } else if prompt.controls.is_empty() {
            prompt = input_prompt(
                title.clone(),
                prompt.message.clone().unwrap_or_default(),
                Vec::new(),
                true,
            );
        }

        let _guard = self.notify_lock.lock().await;
        let row = match self.notifies.by_key(&caller.requester, &key).await? {
            Some(row) => row,
            None => {
                let (sender, zone_host) = self.notify_identity();
                let notify_id = format!("{}{}", NOTIFY_ID_PREFIX, uuid::Uuid::new_v4().simple());
                let row = NotifyRow {
                    url: notify_url(zone_host.as_deref(), &notify_id),
                    notify_id,
                    to_user,
                    owner_did,
                    requester: caller.requester.clone(),
                    idempotency_key: key,
                    task_principal: (!caller.system)
                        .then(|| caller.task_principal.clone())
                        .flatten(),
                    kind: req.kind,
                    category,
                    level: req.level.unwrap_or_default(),
                    title,
                    prompt,
                    source,
                    sender,
                    created_at_ms: Self::now_ms(),
                    expires_at_ms: req.expires_at_ms,
                    state: NotifyRowState::Delivering,
                    task_id: None,
                    task_granted: false,
                    planned: None,
                    home: None,
                    home_unread: false,
                    deliveries: Vec::new(),
                    observed: Vec::new(),
                    acked: false,
                    acked_by: None,
                    closed_status: None,
                    closed_reason: None,
                    closed_at_ms: None,
                };
                self.notifies.insert(&row).await?
            }
        };
        let row = self.continue_send(row).await?;
        drop(_guard);
        self.notify_view(&row).await
    }

    /// Run the steps of a send that are not done yet, saving after each.
    async fn continue_send(&self, mut row: NotifyRow) -> Result<NotifyRow> {
        if row.state != NotifyRowState::Delivering {
            return Ok(row);
        }
        if row.kind == NotifyKind::Input {
            if row.task_id.is_none() {
                let task = self
                    .notify_tasks
                    .get()
                    .create(Self::notify_task_req(&row)?)
                    .await?;
                row.task_id = Some(task.task_id);
                self.notifies.update(&row).await?;
            }
            if !row.task_granted {
                let task_id = row.task_id.clone().unwrap_or_default();
                self.notify_tasks
                    .get()
                    .grant(&task_id, Self::notify_task_grants(&row))
                    .await?;
                row.task_granted = true;
                self.notifies.update(&row).await?;
            }
        }
        if row.planned.is_none() {
            row.planned = Some(self.plan_copies(&row).await?);
            self.notifies.update(&row).await?;
        }
        let planned = row.planned.clone().unwrap_or_default();
        if row.home.is_none() {
            self.ensure_notify_session(&row.owner_did, row.category, &row.sender)
                .await?;
            let unread = match row.level {
                NotifyLevel::Quiet => false,
                NotifyLevel::Urgent => true,
                NotifyLevel::Normal | NotifyLevel::Timely => planned.is_empty(),
            };
            let msg = Self::notify_message(
                &row,
                row.sender.clone(),
                row.owner_did.clone(),
                Some(row.category.session_id()),
                true,
            )?;
            let record = self
                .write_inbox_record(
                    &row.owner_did,
                    msg,
                    if unread {
                        RecipientState::Unread
                    } else {
                        RecipientState::Read
                    },
                    &format!("notify:{}:home", row.notify_id),
                )
                .await?;
            row.home = Some(NotifyRecordRef {
                owner: row.owner_did.to_string(),
                session_id: row.category.session_id(),
                record_id: record.record_id,
                msg_id: record.msg_id.to_string(),
                from: row.sender.to_string(),
            });
            row.home_unread = unread;
            self.notifies.update(&row).await?;
        }
        let agent = row
            .source
            .agent_did
            .as_deref()
            .and_then(|d| DID::from_str(d).ok());
        if let Some(agent) = agent {
            for conversation in planned.iter() {
                if row
                    .deliveries
                    .iter()
                    .any(|d| d.session_id == conversation.session_id)
                {
                    continue;
                }
                let delivery = self.send_copy(&row, &agent, conversation).await?;
                row.deliveries.push(delivery);
                self.notifies.update(&row).await?;
            }
            for delivery in row.deliveries.clone() {
                if !delivery.ok || row.observed.contains(&delivery.session_id) {
                    continue;
                }
                self.write_observation(&row, &agent, &delivery).await?;
                row.observed.push(delivery.session_id.clone());
                self.notifies.update(&row).await?;
            }
        }
        row.state = NotifyRowState::Delivered;
        self.notifies.update(&row).await?;
        info!(
            "notify {} ({} {} {}) for {} delivered: {} copies",
            row.notify_id,
            row.kind.as_str(),
            row.category.as_str(),
            row.level.as_str(),
            row.to_user,
            row.deliveries.len()
        );
        Ok(row)
    }

    fn notify_task_req(row: &NotifyRow) -> Result<CreateTaskReq> {
        let input = NotifyTaskInput {
            notify: NotifyTaskEnvelope {
                notify_id: row.notify_id.clone(),
                kind: row.kind,
                category: row.category,
                level: row.level,
                source: row.source.clone(),
                expires_at_ms: row.expires_at_ms,
                url: row.url.clone(),
            },
            prompt: row.prompt.clone(),
        };
        Ok(CreateTaskReq {
            name: format!("notify: {}", truncate(&row.title, 80)),
            schema_id: HUMAN_INPUT_TASK_SCHEMA_ID.to_string(),
            schema_version: None,
            input: serde_json::to_value(&input).map_err(db_error)?,
            executor: CreateTaskExecutor::HumanSet {
                assignees: vec![row.to_user.clone()],
            },
            parent_id: None,
            child_control_policy: None,
            policy_preset: None,
            permission_boundary: false,
            storage_domain: Some(StorageDomain::User),
            idempotency_key: format!("notify:{}", row.notify_id),
            retry_of: None,
            supersedes: None,
            message: Some(row.title.clone()),
        })
    }

    fn notify_task_grants(row: &NotifyRow) -> Vec<TaskAclGrantSpec> {
        let mut grants = vec![TaskAclGrantSpec {
            subject: TaskGrantSubject::User {
                user_id: row.to_user.clone(),
            },
            actions: vec![TaskAction::ReadResult],
            scope: TaskGrantScope::SelfOnly,
            data_scope: TaskDataScope::Full,
        }];
        if let Some((user_id, app_id)) = row.task_principal.clone() {
            grants.push(TaskAclGrantSpec {
                subject: TaskGrantSubject::Principal { user_id, app_id },
                actions: vec![
                    TaskAction::ReadMeta,
                    TaskAction::ReadInput,
                    TaskAction::ReadResult,
                    TaskAction::Control,
                ],
                scope: TaskGrantScope::SelfOnly,
                data_scope: TaskDataScope::Full,
            });
        }
        grants
    }

    /// Body text for clients that do not know the intent (Telegram).
    fn notify_text(row: &NotifyRow) -> String {
        let mut lines = vec![row.title.clone()];
        if let Some(message) = row
            .prompt
            .message
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty() && *m != row.title)
        {
            lines.push(message.to_string());
        }
        let options = prompt_options(&row.prompt);
        if !options.is_empty() {
            lines.push(format!(
                "选项：{}",
                options
                    .iter()
                    .map(|o| o.label.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
        lines.push(format!("处理：{}", row.url));
        lines.join("\n")
    }

    fn notify_message(
        row: &NotifyRow,
        from: DID,
        to: DID,
        to_session: Option<String>,
        home: bool,
    ) -> Result<MsgObject> {
        let data = NotifyMessageData {
            version: NOTIFY_DATA_VERSION,
            notify_id: row.notify_id.clone(),
            kind: row.kind,
            category: row.category,
            level: row.level,
            home,
            source: row.source.clone(),
            prompt: row.prompt.clone(),
            task_id: row.task_id.clone(),
            url: row.url.clone(),
            expires_at_ms: row.expires_at_ms,
        };
        Ok(MsgObject {
            from,
            to: vec![to],
            to_session,
            kind: MsgObjKind::Notify,
            created_at_ms: row.created_at_ms,
            content: MsgContent {
                title: Some(row.title.clone()),
                format: Some(MsgContentFormat::TextPlain),
                content: Self::notify_text(row),
                machine: Some(machine_content(NOTIFY_INTENT, &data)?),
                refs: Vec::new(),
            },
            ..Default::default()
        })
    }

    async fn ensure_notify_session(
        &self,
        owner: &DID,
        category: NotifyCategory,
        sender: &DID,
    ) -> Result<()> {
        let session_id = category.session_id();
        if let Some(state) = self.msg_box_db.get_owner_session(owner, &session_id).await? {
            if state.registered {
                return Ok(());
            }
        }
        self.create_session_internal(MsgCenterCreateSessionReq {
            owner: owner.clone(),
            peer_did: sender.clone(),
            session_id: Some(session_id),
            title: Some(category_title(category).to_string()),
            binding: None,
            origin: Some(NOTIFY_SESSION_ORIGIN.to_string()),
        })
        .await?;
        Ok(())
    }

    /// A record in `owner`'s INBOX written by msg-center itself: no contact
    /// grading, idempotent by `key`.
    async fn write_inbox_record(
        &self,
        owner: &DID,
        msg: MsgObject,
        state: RecipientState,
        key: &str,
    ) -> Result<MailboxRecord> {
        let msg = self.with_state_write(|s| Ok(Self::ensure_message(s, msg)))?;
        let (msg_id, msg_json) = msg.gen_obj_id();
        self.store_message(&msg_id, &msg_json, &msg, crate::buckets::Arrival::Local)
            .await?;
        let record = Self::build_mailbox_record(
            owner.clone(),
            MailboxKind::Inbox,
            &msg,
            state,
            None,
            Vec::new(),
            "inbox",
        )?;
        let outcome = self
            .msg_box_db
            .commit_dispatch_records(
                IDEMPOTENCY_SCOPE_NOTIFY,
                &owner.to_string(),
                Some(key),
                None,
                &msg_id,
                &msg,
                std::slice::from_ref(&record),
                &record.record_id,
                Self::now_ms(),
                None,
            )
            .await?;
        if let IdempotencyCommitOutcome::Committed = outcome {
            self.note_records_committed(std::slice::from_ref(&record))
                .await;
            Self::publish_box_changed_event(&record, "upsert");
        }
        Ok(record)
    }

    /// The user's one-to-one conversations with `agent`, most recent first:
    /// every Agent INBOX session in which the user spoke, with the reply
    /// coordinates of the user's latest message there.
    pub(crate) async fn agent_conversations(
        &self,
        agent: &DID,
        user: &DID,
    ) -> Result<Vec<Conversation>> {
        let mut out = Vec::new();
        for entry in self.msg_box_db.list_visible_session_index(agent).await? {
            let session_id = entry.session_id;
            let records = self
                .msg_box_db
                .list_visible_session_records(
                    agent,
                    &session_id,
                    CONVERSATION_SCAN_RECORDS,
                    None,
                    None,
                    true,
                    None,
                )
                .await?;
            for record in records {
                if record.box_kind != MailboxKind::Inbox || record.msg_kind != MsgObjKind::Chat {
                    continue;
                }
                let speaker = Self::ingress_principal(record.ingress.as_ref())
                    .unwrap_or_else(|| record.from.clone());
                if &speaker != user {
                    continue;
                }
                let to_session = match self.get_message_internal(record.msg_id.clone()).await {
                    Ok(Some(msg)) => msg.to_session.clone(),
                    _ => (session_id != format!("dm:{}", record.from.to_string()))
                        .then(|| session_id.clone()),
                };
                let platform = match ContactMgr::parse_msgtunnel_did(&record.from) {
                    Some((_, _, instance)) => self
                        .lookup_tunnel_route(&instance)
                        .map(|route| route.platform)
                        .unwrap_or_else(|| "tunnel".to_string()),
                    None => "native".to_string(),
                };
                out.push(Conversation {
                    session_id: session_id.clone(),
                    platform,
                    to: record.from.to_string(),
                    to_session,
                    last_active_at_ms: record.sort_key,
                });
                break;
            }
        }
        out.sort_by(|a, b| {
            b.last_active_at_ms
                .cmp(&a.last_active_at_ms)
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        Ok(out)
    }

    /// Where copies of an Agent's notification go, by level.
    async fn plan_copies(&self, row: &NotifyRow) -> Result<Vec<Conversation>> {
        let Some(agent) = row
            .source
            .agent_did
            .as_deref()
            .and_then(|d| DID::from_str(d).ok())
        else {
            return Ok(Vec::new());
        };
        if row.level == NotifyLevel::Quiet {
            return Ok(Vec::new());
        }
        let candidates = self.agent_conversations(&agent, &row.owner_did).await?;
        let source = row.source.conversation.as_ref().and_then(|c| {
            candidates
                .iter()
                .find(|x| x.to == c.to && x.to_session == c.to_session)
                .cloned()
        });
        let mut planned: Vec<Conversation> = Vec::new();
        match row.level {
            NotifyLevel::Quiet => {}
            NotifyLevel::Normal => planned.extend(source),
            NotifyLevel::Timely => {
                planned.extend(source);
                if let Some(latest) = candidates.first() {
                    if !planned.iter().any(|p| p.session_id == latest.session_id) {
                        planned.push(latest.clone());
                    }
                }
            }
            NotifyLevel::Urgent => planned = candidates,
        }
        Ok(planned)
    }

    async fn send_copy(
        &self,
        row: &NotifyRow,
        agent: &DID,
        conversation: &Conversation,
    ) -> Result<NotifyDelivery> {
        let mut delivery = NotifyDelivery {
            session_id: conversation.session_id.clone(),
            platform: conversation.platform.clone(),
            to: conversation.to.clone(),
            to_session: conversation.to_session.clone(),
            msg_id: None,
            ok: false,
            reason: None,
        };
        let to = match DID::from_str(&conversation.to) {
            Ok(did) => did,
            Err(e) => {
                delivery.reason = Some(format!("invalid endpoint: {e}"));
                return Ok(delivery);
            }
        };
        let msg = Self::notify_message(
            row,
            agent.clone(),
            to,
            conversation.to_session.clone(),
            false,
        )?;
        let key = format!("notify:{}:{}", row.notify_id, conversation.session_id);
        match self.post_send_internal(msg, Some(key)).await {
            Ok(result) => {
                delivery.msg_id = Some(result.msg_id.to_string());
                delivery.ok = result.ok;
                delivery.reason = result.reason;
            }
            Err(e) => delivery.reason = Some(e.to_string()),
        }
        if !delivery.ok {
            warn!(
                "notify {} copy to {} failed: {:?}",
                row.notify_id, conversation.session_id, delivery.reason
            );
        }
        Ok(delivery)
    }

    async fn write_observation(
        &self,
        row: &NotifyRow,
        agent: &DID,
        delivery: &NotifyDelivery,
    ) -> Result<()> {
        let data = NotifyDeliveredData {
            notify_id: row.notify_id.clone(),
            url: row.url.clone(),
            kind: row.kind,
            category: row.category,
            level: row.level,
            title: row.title.clone(),
            message: row.prompt.message.clone(),
            options: prompt_options(&row.prompt),
            source: NotifySource {
                session_id: row.source.session_id.clone(),
                goal_id: row.source.goal_id.clone(),
                operation: row.source.operation.clone(),
                ..Default::default()
            },
            delivered_msg_id: delivery.msg_id.clone(),
            reply_route: NotifyConversation {
                to: delivery.to.clone(),
                to_session: delivery.to_session.clone(),
            },
            at_ms: row.created_at_ms,
        };
        let msg = MsgObject {
            from: agent.clone(),
            to: vec![agent.clone()],
            to_session: Some(delivery.session_id.clone()),
            kind: MsgObjKind::Event,
            created_at_ms: row.created_at_ms,
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: format!("notify {} delivered: {}", row.notify_id, row.title),
                machine: Some(machine_content(NOTIFY_DELIVERED_INTENT, &data)?),
                ..Default::default()
            },
            ..Default::default()
        };
        self.write_inbox_record(
            agent,
            msg,
            RecipientState::Unread,
            &format!("notify-obs:{}:{}", row.notify_id, delivery.session_id),
        )
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // records of a notice
    // ------------------------------------------------------------------

    /// Records the user may read: the home record (when written unread)
    /// and the native copies in the user's INBOX.
    async fn notice_records(&self, row: &NotifyRow, all: bool) -> Result<Vec<MailboxRecord>> {
        let mut ids = Vec::new();
        if let Some(home) = row.home.as_ref() {
            if all || row.home_unread {
                ids.push(home.record_id.clone());
            }
        }
        for delivery in row.deliveries.iter().filter(|d| d.ok && d.platform == "native") {
            let Some(msg_id) = delivery.msg_id.as_deref().and_then(|m| ObjId::new(m).ok()) else {
                continue;
            };
            ids.push(Self::build_record_id(
                &row.owner_did,
                &MailboxKind::Inbox,
                &msg_id,
                "inbox",
            ));
        }
        let mut out = Vec::new();
        for id in ids {
            if let Some(record) = self.msg_box_db.get_record(&row.owner_did, &id).await? {
                out.push(record);
            }
        }
        Ok(out)
    }

    async fn set_notice_records(&self, row: &NotifyRow, state: RecipientState) -> Result<()> {
        for record in self.notice_records(row, true).await? {
            let applies = match state {
                RecipientState::Read => {
                    matches!(record.state, RecipientState::Unread | RecipientState::Reading)
                }
                RecipientState::Archived => !matches!(
                    record.state,
                    RecipientState::Archived | RecipientState::Deleted
                ),
                _ => false,
            };
            if !applies {
                continue;
            }
            if let Err(e) = self.update_record_state_internal(record.clone(), state).await {
                warn!(
                    "notify {}: record {} not set to {:?}: {}",
                    row.notify_id, record.record_id, state, e
                );
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // view, get, list
    // ------------------------------------------------------------------

    pub(crate) async fn notify_view(&self, row: &NotifyRow) -> Result<NotifyView> {
        let mut result = None;
        let status = match (row.kind, row.closed_status) {
            (NotifyKind::Notice, Some(status)) => status,
            (NotifyKind::Notice, None) => {
                let read = row.acked
                    || self
                        .notice_records(row, false)
                        .await?
                        .iter()
                        .any(|r| matches!(r.state, RecipientState::Read | RecipientState::Archived));
                if read {
                    NotifyStatus::Read
                } else {
                    NotifyStatus::Open
                }
            }
            (NotifyKind::Input, closed) => match row.task_id.as_deref() {
                Some(task_id) => match self.notify_tasks.get().get(task_id).await {
                    Ok(task) => {
                        result = notify_task_result(&task);
                        input_status_of(&task, row.closed_reason.as_deref())
                    }
                    Err(e) => {
                        warn!("notify {}: task {task_id} not readable: {e}", row.notify_id);
                        closed.unwrap_or(NotifyStatus::Open)
                    }
                },
                None => closed.unwrap_or(NotifyStatus::Open),
            },
        };
        Ok(NotifyView {
            notify_id: row.notify_id.clone(),
            url: row.url.clone(),
            to_user: row.to_user.clone(),
            kind: row.kind,
            category: row.category,
            level: row.level,
            status,
            prompt: row.prompt.clone(),
            source: row.source.clone(),
            created_at_ms: row.created_at_ms,
            expires_at_ms: row.expires_at_ms,
            closed_at_ms: row.closed_at_ms,
            closed_reason: row.closed_reason.clone(),
            acked_by: row.acked_by.clone(),
            task_id: row.task_id.clone(),
            result,
            home: row.home.clone(),
            deliveries: row.deliveries.clone(),
        })
    }

    async fn notify_row(&self, notify_id: &str) -> Result<NotifyRow> {
        self.notifies
            .get(notify_id.trim())
            .await?
            .ok_or_else(|| RPCErrors::KeyNotExist(format!("notification {notify_id}")))
    }

    async fn readable_row(&self, caller: &NotifyCaller, notify_id: &str) -> Result<NotifyRow> {
        let row = self.notify_row(notify_id).await?;
        if !caller.may_read(&row) {
            // Same answer as for a missing one: no probing other users.
            return Err(RPCErrors::KeyNotExist(format!("notification {notify_id}")));
        }
        Ok(row)
    }

    pub(crate) async fn notify_get(
        &self,
        caller: &NotifyCaller,
        notify_id: &str,
    ) -> Result<NotifyView> {
        let row = self.readable_row(caller, notify_id).await?;
        self.notify_view(&row).await
    }

    pub(crate) async fn notify_list(
        &self,
        caller: &NotifyCaller,
        req: NotifyListReq,
    ) -> Result<NotifyListResult> {
        let owner = match (req.to_user.as_deref(), caller.system) {
            (Some(user), true) => Some(
                self.token_verifier
                    .get()
                    .resolve_user_did(user)
                    .await
                    .map_err(|e| RPCErrors::ParseRequestError(format!("unknown user {user}: {e}")))?,
            ),
            (Some(user), false) if caller.user_id.as_deref() != Some(user) => {
                return Err(denied("may only list the notifications of its own user"))
            }
            (_, true) => None,
            (_, false) => Some(
                caller
                    .user_did
                    .clone()
                    .ok_or_else(|| denied("caller has no user"))?,
            ),
        };
        let limit = req.limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
        let mut cursor = req.cursor.as_deref().and_then(|c| {
            let (at, id) = c.split_once(':')?;
            Some((at.parse::<u64>().ok()?, id.to_string()))
        });
        let mut items = Vec::new();
        let mut next_cursor = None;
        'scan: loop {
            let page = self.notifies.page(owner.as_ref(), cursor.clone(), limit * 2).await?;
            let exhausted = page.len() < limit * 2;
            for row in page {
                cursor = Some((row.created_at_ms, row.notify_id.clone()));
                if !Self::row_matches(&row, &req) {
                    continue;
                }
                let view = self.notify_view(&row).await?;
                if let Some(statuses) = req.status.as_ref() {
                    if !statuses.contains(&view.status) {
                        continue;
                    }
                }
                items.push(view);
                if items.len() == limit {
                    next_cursor = cursor.as_ref().map(|(at, id)| format!("{at}:{id}"));
                    break 'scan;
                }
            }
            if exhausted {
                break;
            }
        }
        Ok(NotifyListResult { items, next_cursor })
    }

    fn row_matches(row: &NotifyRow, req: &NotifyListReq) -> bool {
        req.kind.is_none_or(|k| k == row.kind)
            && req.category.is_none_or(|c| c == row.category)
            && req
                .agent_did
                .as_deref()
                .is_none_or(|a| row.source.agent_did.as_deref() == Some(a))
            && req
                .app_id
                .as_deref()
                .is_none_or(|a| row.source.app_id.as_deref() == Some(a))
            && req
                .session_id
                .as_deref()
                .is_none_or(|s| row.source.session_id.as_deref() == Some(s))
            && req
                .goal_id
                .as_deref()
                .is_none_or(|g| row.source.goal_id.as_deref() == Some(g))
            && req.since_ms.is_none_or(|since| row.created_at_ms >= since)
    }

    // ------------------------------------------------------------------
    // ack, dismiss, cancel
    // ------------------------------------------------------------------

    pub(crate) async fn notify_ack(
        &self,
        caller: &NotifyCaller,
        notify_id: &str,
        on_behalf: Option<String>,
    ) -> Result<NotifyView> {
        let _guard = self.notify_lock.lock().await;
        let mut row = self.readable_row(caller, notify_id).await?;
        if !caller.may_act_for_user(&row) {
            return Err(denied("only the user acknowledges a notification"));
        }
        if row.kind != NotifyKind::Notice {
            return Err(RPCErrors::ParseRequestError(
                "an input is answered through its task, not acknowledged".into(),
            ));
        }
        if row.closed_status.is_none() && !row.acked {
            self.set_notice_records(&row, RecipientState::Read).await?;
            row.acked = true;
            row.acked_by = on_behalf.filter(|s| !s.trim().is_empty());
            self.notifies.update(&row).await?;
        }
        drop(_guard);
        self.notify_view(&row).await
    }

    pub(crate) async fn notify_dismiss(
        &self,
        caller: &NotifyCaller,
        notify_id: &str,
    ) -> Result<NotifyView> {
        let _guard = self.notify_lock.lock().await;
        let mut row = self.readable_row(caller, notify_id).await?;
        if !caller.may_act_for_user(&row) {
            return Err(denied("only the user dismisses a notification"));
        }
        if row.kind != NotifyKind::Notice {
            return Err(RPCErrors::ParseRequestError(
                "an input is dismissed through its task".into(),
            ));
        }
        if row.closed_status.is_none() {
            self.close_notice(&mut row, NotifyStatus::Dismissed, "dismissed")
                .await?;
        }
        drop(_guard);
        self.notify_view(&row).await
    }

    pub(crate) async fn notify_cancel(
        &self,
        caller: &NotifyCaller,
        notify_id: &str,
        reason: Option<String>,
    ) -> Result<NotifyView> {
        let _guard = self.notify_lock.lock().await;
        let mut row = self.readable_row(caller, notify_id).await?;
        if !caller.may_cancel(&row) {
            return Err(denied("only the sender cancels a notification"));
        }
        let reason = reason
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "canceled".to_string());
        self.close_row(&mut row, &reason).await?;
        drop(_guard);
        self.notify_view(&row).await
    }

    async fn close_notice(
        &self,
        row: &mut NotifyRow,
        status: NotifyStatus,
        reason: &str,
    ) -> Result<()> {
        self.set_notice_records(row, RecipientState::Archived)
            .await?;
        row.closed_status = Some(status);
        row.closed_reason = Some(reason.to_string());
        row.closed_at_ms = Some(Self::now_ms());
        row.state = NotifyRowState::Closed;
        self.notifies.update(row).await
    }

    /// Cancel (`reason = expired` → expired): a notice is archived, an
    /// input's task is canceled. A row already closed stays as it is.
    async fn close_row(&self, row: &mut NotifyRow, reason: &str) -> Result<()> {
        if row.state == NotifyRowState::Closed {
            return Ok(());
        }
        match row.kind {
            NotifyKind::Notice => {
                let status = if reason == "expired" {
                    NotifyStatus::Expired
                } else {
                    NotifyStatus::Canceled
                };
                self.close_notice(row, status, reason).await
            }
            NotifyKind::Input => {
                if let Some(task_id) = row.task_id.clone() {
                    let task = self.notify_tasks.get().get(&task_id).await.ok();
                    if task.as_ref().is_some_and(|t| t.phase.is_terminal()) {
                        // Answered (or closed) before the cancel: that stays.
                        row.closed_at_ms = task.and_then(|t| t.completed_at).or(Some(Self::now_ms()));
                    } else {
                        self.notify_tasks
                            .get()
                            .cancel(&task_id, &format!("notify-{}-{reason}", row.notify_id))
                            .await?;
                        row.closed_reason = Some(reason.to_string());
                        row.closed_at_ms = Some(Self::now_ms());
                    }
                } else {
                    row.closed_status = Some(if reason == "expired" {
                        NotifyStatus::Expired
                    } else {
                        NotifyStatus::Canceled
                    });
                    row.closed_reason = Some(reason.to_string());
                    row.closed_at_ms = Some(Self::now_ms());
                }
                row.state = NotifyRowState::Closed;
                self.notifies.update(row).await
            }
        }
    }

    // ------------------------------------------------------------------
    // sweep
    // ------------------------------------------------------------------

    /// Finish interrupted sends, expire, and close rows whose task ended.
    pub(crate) async fn sweep_notifies(&self) -> Result<()> {
        let _guard = self.notify_lock.lock().await;
        let now = Self::now_ms();
        for row in self.notifies.unclosed().await? {
            let id = row.notify_id.clone();
            let outcome: Result<()> = async {
                let mut row = self.continue_send(row).await?;
                if row.expires_at_ms.is_some_and(|at| at <= now) {
                    return self.close_row(&mut row, "expired").await;
                }
                if row.kind == NotifyKind::Input {
                    if let Some(task_id) = row.task_id.as_deref() {
                        let task = self.notify_tasks.get().get(task_id).await?;
                        if task.phase.is_terminal() {
                            row.state = NotifyRowState::Closed;
                            row.closed_at_ms = task.completed_at.or(Some(now));
                            self.notifies.update(&row).await?;
                        }
                    }
                }
                Ok(())
            }
            .await;
            if let Err(e) = outcome {
                warn!("notify sweep of {id} failed: {e}");
            }
        }
        Ok(())
    }

    pub fn start_notify_sweep(&self) {
        let center = self.clone();
        tokio::spawn(async move {
            let period = std::time::Duration::from_secs(NOTIFY_SWEEP_INTERVAL_SECS);
            loop {
                tokio::time::sleep(period).await;
                if let Err(e) = center.sweep_notifies().await {
                    warn!("notify sweep failed: {e}");
                }
            }
        });
    }
}

pub(crate) fn notify_identity_slot() -> Arc<RwLock<Option<(DID, Option<String>)>>> {
    Arc::new(RwLock::new(None))
}
