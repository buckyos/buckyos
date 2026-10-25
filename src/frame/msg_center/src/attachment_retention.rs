//! Message attachments are held in the node named store while a message that
//! references them is valid (share design D17).
//!
//! Each accepted message pins its `data_obj` refs under `msg-center:<msg_id>`
//! before the mailbox records are committed. `msg_attachment_retentions`
//! records `pending` → `held`; a group redaction (recall, deleted session)
//! marks the rows `releasing` in its transaction and the reconcile unpins
//! them. Private messages have no server-side delete, so their attachments
//! stay held: one owner hiding or deleting a local view is not the end of the
//! message. The reconcile also finishes interrupted accepts, backfills
//! messages accepted before this table existed (once) and records attachments
//! that cannot be read here in `msg_attachment_issues`.

use crate::group_store::db_error;
use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use async_trait::async_trait;
use buckyos_api::get_buckyos_api_runtime;
use kRPC::RPCErrors;
use named_store::{NamedDataMgr, PinScope};
use ndn_lib::{MsgObjKind, MsgObject, NdnError, ObjId, RefTarget};
use serde::Serialize;
use sqlx::Row;
use std::sync::{Arc, RwLock};

type Result<T> = std::result::Result<T, RPCErrors>;

pub const RETENTION_SWEEP_INTERVAL_SECS: u64 = 60;
/// An accept pins before it commits; younger `pending` rows may still be in flight.
pub const PENDING_GRACE_MS: u64 = 10 * 60 * 1000;
const BACKFILL_MARKER: &str = "backfill_v1";

pub fn attachment_owner(msg_id: &ObjId) -> String {
    format!("msg-center:{}", msg_id.to_string())
}

pub fn attachment_ids(msg: &MsgObject) -> Vec<ObjId> {
    let mut ids: Vec<ObjId> = Vec::new();
    for r in &msg.content.refs {
        if let RefTarget::DataObj { obj_id, .. } = &r.target {
            if !ids.contains(obj_id) {
                ids.push(obj_id.clone());
            }
        }
    }
    ids
}

#[async_trait]
pub trait AttachmentStore: Send + Sync {
    async fn retain(&self, obj_id: &ObjId, owner: &str) -> Result<()>;
    async fn release(&self, obj_id: &ObjId, owner: &str) -> Result<()>;
    /// Why a held attachment cannot be read here, if it cannot.
    async fn issue(&self, obj_id: &ObjId) -> Option<String>;
}

fn store_error(e: NdnError) -> RPCErrors {
    RPCErrors::ReasonError(format!("named store: {e}"))
}

#[async_trait]
impl AttachmentStore for NamedDataMgr {
    async fn retain(&self, obj_id: &ObjId, owner: &str) -> Result<()> {
        self.pin(obj_id, owner, PinScope::Recursive, None).await.map_err(store_error)
    }

    async fn release(&self, obj_id: &ObjId, owner: &str) -> Result<()> {
        match self.unpin(obj_id, owner).await {
            Ok(()) | Err(NdnError::NotFound(_)) => Ok(()),
            Err(e) => Err(store_error(e)),
        }
    }

    async fn issue(&self, obj_id: &ObjId) -> Option<String> {
        match self.is_object_stored(obj_id, None).await {
            Ok(true) => None,
            Ok(false) => Some("content_missing".into()),
            Err(error) => Some(format!("object_unavailable: {error}")),
        }
    }
}

/// The zone runtime's named store.
pub struct RuntimeAttachmentStore;

impl RuntimeAttachmentStore {
    async fn store() -> Result<NamedDataMgr> {
        get_buckyos_api_runtime()?.get_local_named_store().await
    }
}

#[async_trait]
impl AttachmentStore for RuntimeAttachmentStore {
    async fn retain(&self, obj_id: &ObjId, owner: &str) -> Result<()> {
        Self::store().await?.retain(obj_id, owner).await
    }

    async fn release(&self, obj_id: &ObjId, owner: &str) -> Result<()> {
        Self::store().await?.release(obj_id, owner).await
    }

    async fn issue(&self, obj_id: &ObjId) -> Option<String> {
        match Self::store().await {
            Ok(store) => store.issue(obj_id).await,
            Err(e) => Some(format!("store_unavailable: {e}")),
        }
    }
}

/// Unset in unit tests that do not exercise attachments; the service sets
/// the runtime store at startup.
#[derive(Default)]
pub struct AttachmentStoreSlot(RwLock<Option<Arc<dyn AttachmentStore>>>);

impl AttachmentStoreSlot {
    pub fn get(&self) -> Option<Arc<dyn AttachmentStore>> {
        self.0.read().unwrap().clone()
    }

    pub fn set(&self, store: Arc<dyn AttachmentStore>) {
        *self.0.write().unwrap() = Some(store);
    }
}

impl std::fmt::Debug for AttachmentStoreSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AttachmentStoreSlot")
    }
}

pub async fn open_tables(db: &MsgBoxDbMgr) -> Result<()> {
    for ddl in [
        "CREATE TABLE IF NOT EXISTS msg_attachment_retentions (msg_id TEXT NOT NULL, obj_id TEXT NOT NULL, state TEXT NOT NULL, updated_at_ms BIGINT NOT NULL, PRIMARY KEY(msg_id, obj_id))",
        "CREATE INDEX IF NOT EXISTS idx_msg_attachment_retentions_state ON msg_attachment_retentions(state)",
        "CREATE TABLE IF NOT EXISTS msg_attachment_issues (msg_id TEXT NOT NULL, obj_id TEXT NOT NULL, issue TEXT NOT NULL, checked_at_ms BIGINT NOT NULL, PRIMARY KEY(msg_id, obj_id))",
        "CREATE TABLE IF NOT EXISTS msg_attachment_meta (name TEXT PRIMARY KEY, value TEXT NOT NULL)",
    ] {
        sqlx::query(ddl).execute(db.pool()).await.map_err(db_error)?;
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct AttachmentReport {
    pub backfilled_messages: usize,
    pub pinned: usize,
    pub released: usize,
    pub failed: usize,
    /// `(msg_id, obj_id, issue)` for held attachments that cannot be read here.
    pub issues: Vec<(String, String, String)>,
}

fn now_ms() -> i64 {
    buckyos_kit::buckyos_get_unix_timestamp() as i64 * 1000
}

impl MessageCenter {
    /// Authorize user-originated references before creating any message or hold.
    /// Signed remote ingress and trusted in-process sends use their existing path.
    pub(crate) async fn authorize_outgoing_attachments(&self, ctx: &kRPC::RPCContext, msg: &MsgObject) -> Result<()> {
        let ids = attachment_ids(msg);
        if ids.is_empty() { return Ok(()); }
        let Some(caller) = self.caller_identity(ctx).await? else { return Ok(()); };
        Self::refuse_guest_attachments(Some(&caller), msg)?;
        let token = ctx.token.as_deref().ok_or_else(|| RPCErrors::NoPermission("attachment token required".into()))?;
        for id in ids {
            if self.authorize_attachment(ctx, &id, None).await.is_ok() { continue; }
            // A group reference may be reused only while its original message
            // remains readable to this caller (membership, history and recall).
            let sql = self.msg_box_db.render_sql("SELECT obj_id,body_json FROM group_objects WHERE body_json LIKE ?");
            let rows = sqlx::query(&sql).bind(format!("%{}%", id.to_string()))
                .fetch_all(self.msg_box_db.pool()).await.map_err(db_error)?;
            let mut readable = false;
            for row in rows {
                let body: String = row.try_get("body_json").map_err(db_error)?;
                let source: MsgObject = serde_json::from_str(&body).map_err(db_error)?;
                if !Self::message_references(&source, &id) { continue; }
                let raw: String = row.try_get("obj_id").map_err(db_error)?;
                let source_id = ObjId::new(&raw).map_err(db_error)?;
                if self.authorize_group_message(&self.group_actor(ctx).await?, &source_id).await.is_ok() { readable = true; break; }
            }
            if readable { continue; }
            if !self.token_verifier.get().authorize_imported_attachment(token, &id).await? {
                return Err(RPCErrors::NoPermission("attachment is not readable by this caller".into()));
            }
        }
        Ok(())
    }

    pub fn set_attachment_store(&self, store: Arc<dyn AttachmentStore>) {
        self.attachment_store.set(store);
    }

    async fn set_retention_state(&self, msg_id: &str, obj_id: &str, state: &str) -> Result<()> {
        let db = &self.msg_box_db;
        let sql = db.render_sql(
            "INSERT INTO msg_attachment_retentions(msg_id,obj_id,state,updated_at_ms) VALUES(?,?,?,?) ON CONFLICT(msg_id,obj_id) DO UPDATE SET state=excluded.state,updated_at_ms=excluded.updated_at_ms",
        );
        sqlx::query(&sql)
            .bind(msg_id)
            .bind(obj_id)
            .bind(state)
            .bind(now_ms())
            .execute(db.pool())
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn retention_rows(&self, msg_id: &str) -> Result<Vec<(String, String)>> {
        let db = &self.msg_box_db;
        let sql = db.render_sql("SELECT obj_id, state FROM msg_attachment_retentions WHERE msg_id=?");
        sqlx::query(&sql)
            .bind(msg_id)
            .fetch_all(db.pool())
            .await
            .map_err(db_error)?
            .iter()
            .map(|r| Ok((r.try_get("obj_id").map_err(db_error)?, r.try_get("state").map_err(db_error)?)))
            .collect()
    }

    async fn count(&self, sql: &str, id: &str) -> Result<i64> {
        let db = &self.msg_box_db;
        sqlx::query(&db.render_sql(sql))
            .bind(id)
            .fetch_one(db.pool())
            .await
            .map_err(db_error)?
            .try_get("n")
            .map_err(db_error)
    }

    /// Some owner still references the message and it was not redacted.
    async fn message_live(&self, msg_id: &str) -> Result<bool> {
        let referenced = self.count("SELECT COUNT(*) AS n FROM msg_refs WHERE msg_id=?", msg_id).await?;
        let redacted = self
            .count("SELECT COUNT(*) AS n FROM group_objects WHERE obj_id=? AND body_json IS NULL", msg_id)
            .await?;
        Ok(referenced > 0 && redacted == 0)
    }

    /// Holds the attachments of `msg` before its records are committed; an
    /// error must fail the accept.
    pub(crate) async fn retain_attachments(&self, msg_id: &ObjId, msg: &MsgObject) -> Result<()> {
        let Some(store) = self.attachment_store.get() else { return Ok(()) };
        let ids = attachment_ids(msg);
        if ids.is_empty() {
            return Ok(());
        }
        let key = msg_id.to_string();
        let rows = self.retention_rows(&key).await?;
        let owner = attachment_owner(msg_id);
        for id in &ids {
            let id_s = id.to_string();
            if rows.iter().any(|(o, s)| o == &id_s && s == "held") {
                continue;
            }
            self.set_retention_state(&key, &id_s, "pending").await?;
            store.retain(id, &owner).await?;
        }
        Ok(())
    }

    /// The records are committed: the pins of `msg_id` are now owned by it.
    pub(crate) async fn confirm_attachments(&self, msg_id: &ObjId) {
        let db = &self.msg_box_db;
        let sql = db.render_sql("UPDATE msg_attachment_retentions SET state='held',updated_at_ms=? WHERE msg_id=? AND state='pending'");
        if let Err(e) = sqlx::query(&sql).bind(now_ms()).bind(msg_id.to_string()).execute(db.pool()).await {
            log::warn!("confirm attachment holds of {} failed (reconcile retries): {}", msg_id, e);
        }
    }

    /// The message was not accepted: release what `retain_attachments` took.
    pub(crate) async fn drop_unaccepted_attachments(&self, msg_id: &ObjId) {
        let Some(store) = self.attachment_store.get() else { return };
        let mut report = AttachmentReport::default();
        if let Err(e) = self.settle_message(store.as_ref(), &msg_id.to_string(), &mut report).await {
            log::warn!("release attachment holds of rejected {} failed (reconcile retries): {}", msg_id, e);
        }
    }

    /// Brings one message's pins to what it should hold now.
    async fn settle_message(&self, store: &dyn AttachmentStore, msg_id: &str, report: &mut AttachmentReport) -> Result<()> {
        let id = ObjId::new(msg_id).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        let owner = attachment_owner(&id);
        let live = self.message_live(msg_id).await?;
        let mut first_err = None;
        for (obj, state) in self.retention_rows(msg_id).await? {
            let obj_id = ObjId::new(&obj).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
            if live && state != "releasing" {
                if state == "held" {
                    continue;
                }
                match store.retain(&obj_id, &owner).await {
                    Ok(()) => {
                        self.set_retention_state(msg_id, &obj, "held").await?;
                        report.pinned += 1;
                    }
                    Err(e) => first_err = first_err.or(Some(e)),
                }
                continue;
            }
            self.set_retention_state(msg_id, &obj, "releasing").await?;
            match store.release(&obj_id, &owner).await {
                Ok(()) => {
                    let db = &self.msg_box_db;
                    for sql in [
                        "DELETE FROM msg_attachment_retentions WHERE msg_id=? AND obj_id=?",
                        "DELETE FROM msg_attachment_issues WHERE msg_id=? AND obj_id=?",
                    ] {
                        sqlx::query(&db.render_sql(sql))
                            .bind(msg_id)
                            .bind(&obj)
                            .execute(db.pool())
                            .await
                            .map_err(db_error)?;
                    }
                    report.released += 1;
                }
                Err(e) => first_err = first_err.or(Some(e)),
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    async fn msg_ids(&self, sql: &str, bind: Option<i64>) -> Result<Vec<String>> {
        let db = &self.msg_box_db;
        let sql = db.render_sql(sql);
        let mut q = sqlx::query(&sql);
        if let Some(v) = bind {
            q = q.bind(v);
        }
        q.fetch_all(db.pool())
            .await
            .map_err(db_error)?
            .iter()
            .map(|r| r.try_get("msg_id").map_err(db_error))
            .collect()
    }

    /// Messages accepted before holds were recorded: pin what they reference (once).
    async fn backfill_attachments(&self, store: &dyn AttachmentStore, report: &mut AttachmentReport) -> Result<()> {
        let db = &self.msg_box_db;
        let marker = db.render_sql("SELECT COUNT(*) AS n FROM msg_attachment_meta WHERE name=?");
        let done: i64 = sqlx::query(&marker)
            .bind(BACKFILL_MARKER)
            .fetch_one(db.pool())
            .await
            .map_err(db_error)?
            .try_get("n")
            .map_err(db_error)?;
        if done > 0 {
            return Ok(());
        }
        let mut complete = true;
        for msg_id in self.msg_ids("SELECT DISTINCT msg_id FROM msg_refs", None).await? {
            if !self.retention_rows(&msg_id).await?.is_empty() || !self.message_live(&msg_id).await? {
                continue;
            }
            let Ok(id) = ObjId::new(&msg_id) else { continue };
            let msg = match self.load_message(&id).await {
                Ok(msg) => msg,
                Err(e) => {
                    self.record_issue(&msg_id, "", &format!("message_unreadable: {e}")).await?;
                    report.issues.push((msg_id.clone(), String::new(), "message_unreadable".into()));
                    continue;
                }
            };
            // Joined groups keep their attachments in the hosting zone.
            if msg.kind == MsgObjKind::GroupMsg {
                let hosted = match msg.to.first() {
                    Some(group) => self.groups.load(group).await?.is_some(),
                    None => false,
                };
                if !hosted {
                    continue;
                }
            }
            let ids = attachment_ids(&msg);
            if ids.is_empty() {
                continue;
            }
            report.backfilled_messages += 1;
            let owner = attachment_owner(&id);
            for obj in ids {
                self.set_retention_state(&msg_id, &obj.to_string(), "pending").await?;
                match store.retain(&obj, &owner).await {
                    Ok(()) => {
                        self.set_retention_state(&msg_id, &obj.to_string(), "held").await?;
                        report.pinned += 1;
                    }
                    Err(e) => {
                        complete = false;
                        report.failed += 1;
                        log::warn!("backfill hold of {} for {} failed: {}", obj, msg_id, e);
                    }
                }
            }
        }
        if complete {
            let sql = db.render_sql("INSERT INTO msg_attachment_meta(name,value) VALUES(?,?) ON CONFLICT(name) DO NOTHING");
            sqlx::query(&sql)
                .bind(BACKFILL_MARKER)
                .bind(now_ms().to_string())
                .execute(db.pool())
                .await
                .map_err(db_error)?;
        }
        Ok(())
    }

    async fn record_issue(&self, msg_id: &str, obj_id: &str, issue: &str) -> Result<()> {
        let db = &self.msg_box_db;
        let sql = db.render_sql(
            "INSERT INTO msg_attachment_issues(msg_id,obj_id,issue,checked_at_ms) VALUES(?,?,?,?) ON CONFLICT(msg_id,obj_id) DO UPDATE SET issue=excluded.issue,checked_at_ms=excluded.checked_at_ms",
        );
        sqlx::query(&sql)
            .bind(msg_id)
            .bind(obj_id)
            .bind(issue)
            .bind(now_ms())
            .execute(db.pool())
            .await
            .map_err(db_error)?;
        Ok(())
    }

    /// Finish interrupted accepts and releases, backfill older messages and
    /// check that held attachments are still readable here.
    pub async fn reconcile_attachments(&self, pending_grace_ms: u64) -> Result<AttachmentReport> {
        let mut report = AttachmentReport::default();
        let Some(store) = self.attachment_store.get() else { return Ok(report) };
        self.backfill_attachments(store.as_ref(), &mut report).await?;
        let mut todo = self
            .msg_ids(
                "SELECT DISTINCT msg_id FROM msg_attachment_retentions WHERE state='releasing' OR (state='pending' AND updated_at_ms<=?)",
                Some(now_ms() - pending_grace_ms as i64),
            )
            .await?;
        todo.extend(
            self.msg_ids(
                "SELECT DISTINCT r.msg_id AS msg_id FROM msg_attachment_retentions r JOIN group_objects g ON g.obj_id=r.msg_id WHERE g.body_json IS NULL",
                None,
            )
            .await?,
        );
        todo.sort();
        todo.dedup();
        for msg_id in todo {
            if let Err(e) = self.settle_message(store.as_ref(), &msg_id, &mut report).await {
                report.failed += 1;
                log::warn!("attachment holds of {} not settled: {}", msg_id, e);
            }
        }
        let db = &self.msg_box_db;
        let held = sqlx::query(&db.render_sql("SELECT msg_id, obj_id FROM msg_attachment_retentions WHERE state='held'"))
            .fetch_all(db.pool())
            .await
            .map_err(db_error)?;
        for row in held {
            let msg_id: String = row.try_get("msg_id").map_err(db_error)?;
            let obj: String = row.try_get("obj_id").map_err(db_error)?;
            let Ok(obj_id) = ObjId::new(&obj) else { continue };
            match store.issue(&obj_id).await {
                Some(issue) => {
                    self.record_issue(&msg_id, &obj, &issue).await?;
                    log::warn!("held attachment {} of {} is not readable: {}", obj, msg_id, issue);
                    report.issues.push((msg_id, obj, issue));
                }
                None => {
                    sqlx::query(&db.render_sql("DELETE FROM msg_attachment_issues WHERE msg_id=? AND obj_id=?"))
                        .bind(&msg_id)
                        .bind(&obj)
                        .execute(db.pool())
                        .await
                        .map_err(db_error)?;
                }
            }
        }
        Ok(report)
    }

    pub fn start_attachment_retention(&self) {
        let center = self.clone();
        tokio::spawn(async move {
            loop {
                match center.reconcile_attachments(PENDING_GRACE_MS).await {
                    Ok(r) if r.pinned + r.released + r.failed > 0 || !r.issues.is_empty() => {
                        log::info!("attachment retention: {:?}", r)
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("attachment retention reconcile failed: {}", e),
                }
                tokio::time::sleep(std::time::Duration::from_secs(RETENTION_SWEEP_INTERVAL_SECS)).await;
            }
        });
    }
}
