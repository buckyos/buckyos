use crate::group_types::*;
use crate::msg_box_db::MsgBoxDbMgr;
use buckyos_api::{DeliveryRecord, MailboxRecord};
use name_lib::DID;
use ndn_lib::{MsgObject, NamedObject, ObjId};
use sqlx::{Any, Row, Transaction};

pub fn db_error(e: impl std::fmt::Display) -> kRPC::RPCErrors {
    kRPC::RPCErrors::ReasonError(format!("group-db:{e}"))
}
#[derive(Clone, Debug)]
pub struct GroupStore {
    pub db: MsgBoxDbMgr,
    pub sync_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub read_hook_cache:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>>,
}
pub struct GroupTransaction<'a> {
    pub tx: Transaction<'a, Any>,
    pub state: GroupState,
    pub store: GroupStore,
    /// Mailbox records written in this transaction; their `box_changed`
    /// events are published once the transaction has committed, so group
    /// projections notify clients exactly like ordinary deliveries.
    pub projections: Vec<MailboxRecord>,
}
impl GroupStore {
    pub async fn open(db: MsgBoxDbMgr) -> Result<Self> {
        for ddl in [
            "CREATE TABLE IF NOT EXISTS group_states (group_did TEXT PRIMARY KEY, state_json TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS group_objects (obj_id TEXT PRIMARY KEY, group_did TEXT NOT NULL, body_json TEXT, jwt TEXT)",
            "CREATE INDEX IF NOT EXISTS idx_group_objects_group ON group_objects(group_did)",
            "CREATE TABLE IF NOT EXISTS group_sync_tokens (token TEXT PRIMARY KEY, group_did TEXT NOT NULL, reader_did TEXT NOT NULL, group_seq BIGINT NOT NULL, created_at_ms BIGINT NOT NULL)",
            "CREATE INDEX IF NOT EXISTS idx_group_sync_reader ON group_sync_tokens(group_did, reader_did)",
            "CREATE TABLE IF NOT EXISTS group_create_operations (actor TEXT NOT NULL, operation_key TEXT NOT NULL, request_json TEXT NOT NULL, result_json TEXT NOT NULL, PRIMARY KEY(actor,operation_key))",
            "CREATE TABLE IF NOT EXISTS group_locks (lock_key TEXT PRIMARY KEY, schema_version BIGINT NOT NULL)",
            "INSERT INTO group_locks(lock_key,schema_version) VALUES ('create',1) ON CONFLICT(lock_key) DO NOTHING",
            "CREATE TABLE IF NOT EXISTS group_object_purges (obj_id TEXT PRIMARY KEY, schema_version BIGINT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS group_doc_publications (group_did TEXT PRIMARY KEY, generation BIGINT NOT NULL, published_generation BIGINT NOT NULL DEFAULT 0, document_status TEXT NOT NULL, document_json TEXT)",
            "CREATE TABLE IF NOT EXISTS joined_groups (owner TEXT NOT NULL, group_did TEXT NOT NULL, state_json TEXT NOT NULL, PRIMARY KEY(owner, group_did))",
        ] { sqlx::query(ddl).execute(db.pool()).await.map_err(db_error)?; }
        Ok(Self {
            db,
            sync_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            read_hook_cache: Default::default(),
        })
    }
    pub async fn load(&self, d: &DID) -> Result<Option<GroupState>> {
        let sql = self
            .db
            .render_sql("SELECT state_json FROM group_states WHERE group_did=?");
        sqlx::query(&sql)
            .bind(d.to_string())
            .fetch_optional(self.db.pool())
            .await
            .map_err(db_error)?
            .map(|r| {
                let body: String = r.try_get("state_json").map_err(db_error)?;
                serde_json::from_str(&body).map_err(db_error)
            })
            .transpose()
    }
    pub async fn list(&self) -> Result<Vec<GroupState>> {
        sqlx::query("SELECT state_json FROM group_states ORDER BY group_did")
            .fetch_all(self.db.pool())
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|r| {
                let body: String = r.try_get("state_json").map_err(db_error)?;
                serde_json::from_str(&body).map_err(db_error)
            })
            .collect()
    }
    pub async fn begin(&self, d: &DID) -> Result<GroupTransaction<'_>> {
        let mut tx = self.db.pool().begin().await.map_err(db_error)?;
        let sql = self
            .db
            .render_sql("UPDATE group_states SET state_json=state_json WHERE group_did=?");
        if sqlx::query(&sql)
            .bind(d.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db_error)?
            .rows_affected()
            == 0
        {
            return Err(missing());
        }
        let sql = self
            .db
            .render_sql("SELECT state_json FROM group_states WHERE group_did=?");
        let row = sqlx::query(&sql)
            .bind(d.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
        let body: String = row.try_get("state_json").map_err(db_error)?;
        Ok(GroupTransaction {
            tx,
            state: serde_json::from_str(&body).map_err(db_error)?,
            store: self.clone(),
            projections: vec![],
        })
    }
    pub async fn object(
        &self,
        id: &ObjId,
    ) -> Result<Option<(DID, Option<String>, Option<String>)>> {
        let sql = self
            .db
            .render_sql("SELECT group_did,body_json,jwt FROM group_objects WHERE obj_id=?");
        sqlx::query(&sql)
            .bind(id.to_string())
            .fetch_optional(self.db.pool())
            .await
            .map_err(db_error)?
            .map(|r| {
                let d: String = r.try_get("group_did").map_err(db_error)?;
                Ok((
                    DID::from_str(&d).map_err(invalid)?,
                    r.try_get("body_json").map_err(db_error)?,
                    r.try_get("jwt").map_err(db_error)?,
                ))
            })
            .transpose()
    }
    pub async fn cursor(&self, token: &str, g: &DID, reader: &DID) -> Result<Option<u64>> {
        let sql=self.db.render_sql("SELECT group_seq,created_at_ms FROM group_sync_tokens WHERE token=? AND group_did=? AND reader_did=?");
        sqlx::query(&sql)
            .bind(token)
            .bind(g.to_string())
            .bind(reader.to_string())
            .fetch_optional(self.db.pool())
            .await
            .map_err(db_error)?
            .map(|r| {
                let created = r.try_get::<i64, _>("created_at_ms").map_err(db_error)?;
                if crate::msg_center::MessageCenter::now_ms().saturating_sub(created as u64)
                    > 7 * 86_400_000
                {
                    Ok(None)
                } else {
                    r.try_get::<i64, _>("group_seq")
                        .map(|n| Some(n as u64))
                        .map_err(db_error)
                }
            })
            .transpose()
            .map(Option::flatten)
    }
    pub async fn save_cursor(&self, g: &DID, reader: &DID, seq: u64, now: u64) -> Result<String> {
        let expired = self
            .db
            .render_sql("DELETE FROM group_sync_tokens WHERE created_at_ms<?");
        sqlx::query(&expired)
            .bind(now.saturating_sub(7 * 86_400_000) as i64)
            .execute(self.db.pool())
            .await
            .map_err(db_error)?;
        let token = revision();
        let sql=self.db.render_sql("INSERT INTO group_sync_tokens(token,group_did,reader_did,group_seq,created_at_ms) VALUES (?,?,?,?,?)");
        sqlx::query(&sql)
            .bind(&token)
            .bind(g.to_string())
            .bind(reader.to_string())
            .bind(seq as i64)
            .bind(now as i64)
            .execute(self.db.pool())
            .await
            .map_err(db_error)?;
        Ok(token)
    }
}
impl GroupTransaction<'_> {
    pub async fn object(&mut self, msg: &MsgObject, jwt: Option<&str>) -> Result<()> {
        let (id, body) = msg.gen_obj_id();
        let sql=self.store.db.render_sql("INSERT INTO group_objects(obj_id,group_did,body_json,jwt) VALUES(?,?,?,?) ON CONFLICT(obj_id) DO UPDATE SET jwt=COALESCE(group_objects.jwt,excluded.jwt)");
        sqlx::query(&sql)
            .bind(id.to_string())
            .bind(self.state.group_did.to_string())
            .bind(body)
            .bind(jwt)
            .execute(&mut *self.tx)
            .await
            .map_err(db_error)?;
        if let Some(jwt) = jwt {
            let sql=self.store.db.render_sql("INSERT INTO msg_jwt_originals(msg_id,jwt,created_at_ms) VALUES(?,?,?) ON CONFLICT(msg_id) DO NOTHING");
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(jwt)
                .bind(self.state.accepted_at_ms as i64)
                .execute(&mut *self.tx)
                .await
                .map_err(db_error)?;
        }
        Ok(())
    }
    pub async fn mailbox(&mut self, record: &MailboxRecord, msg: &MsgObject) -> Result<()> {
        self.store
            .db
            .upsert_record_with_msg_tx(&mut self.tx, record, Some(msg))
            .await?;
        self.projections.push(record.clone());
        Ok(())
    }
    pub async fn delivery(&mut self, record: &DeliveryRecord) -> Result<()> {
        self.store
            .db
            .create_delivery_if_absent_tx(&mut self.tx, record)
            .await
    }
    pub async fn redact(&mut self, id: &ObjId) -> Result<()> {
        let purge=self.store.db.render_sql("INSERT INTO group_object_purges(obj_id,schema_version) VALUES(?,1) ON CONFLICT(obj_id) DO NOTHING");
        sqlx::query(&purge)
            .bind(id.to_string())
            .execute(&mut *self.tx)
            .await
            .map_err(db_error)?;
        for stmt in [
            "UPDATE group_objects SET body_json=NULL,jwt=NULL WHERE obj_id=?",
            "DELETE FROM msg_jwt_originals WHERE msg_id=?",
            "UPDATE delivery_records SET state='DEAD' WHERE msg_id=? AND state!='SENT'",
        ] {
            let sql = self.store.db.render_sql(stmt);
            sqlx::query(&sql)
                .bind(id.to_string())
                .execute(&mut *self.tx)
                .await
                .map_err(db_error)?;
        }
        if let Some(m) = self.state.messages.get_mut(&id.to_string()) {
            m.redacted = true;
        }
        Ok(())
    }
    pub async fn delete_session_objects(&mut self, s: Option<&str>) -> Result<()> {
        let ids: Vec<_> = self
            .state
            .messages
            .values()
            .filter(|m| m.session_id.as_deref() == s)
            .map(|m| m.obj_id.clone())
            .collect();
        for id in ids {
            let sql = self.store.db.render_sql(
                "DELETE FROM mailbox_records WHERE owner=? AND box_kind='GROUP_INBOX' AND msg_id=?",
            );
            sqlx::query(&sql)
                .bind(self.state.group_did.to_string())
                .bind(id.to_string())
                .execute(&mut *self.tx)
                .await
                .map_err(db_error)?;
            let sql = self
                .store
                .db
                .render_sql("SELECT COUNT(*) AS n FROM mailbox_records WHERE msg_id=?");
            let refs: i64 = sqlx::query(&sql)
                .bind(id.to_string())
                .fetch_one(&mut *self.tx)
                .await
                .map_err(db_error)?
                .try_get("n")
                .map_err(db_error)?;
            if refs == 0 {
                self.redact(&id).await?;
            }
            self.state.messages.remove(&id.to_string());
        }
        Ok(())
    }
    pub async fn commit(mut self) -> Result<()> {
        let status = if self.state.lifecycle == "deleted" {
            "tombstoned"
        } else {
            "active"
        };
        let doc = if status == "active" {
            Some(self.state.public_doc()?.to_string())
        } else {
            None
        };
        let sql = self.store.db.render_sql("INSERT INTO group_doc_publications(group_did,generation,published_generation,document_status,document_json) VALUES(?,?,0,?,?) ON CONFLICT(group_did) DO UPDATE SET generation=excluded.generation,document_status=excluded.document_status,document_json=excluded.document_json WHERE group_doc_publications.document_status<>excluded.document_status OR COALESCE(group_doc_publications.document_json,'')<>COALESCE(excluded.document_json,'')");
        sqlx::query(&sql)
            .bind(self.state.group_did.to_string())
            .bind(self.state.group_seq as i64)
            .bind(status)
            .bind(doc)
            .execute(&mut *self.tx)
            .await
            .map_err(db_error)?;
        let sql = self
            .store
            .db
            .render_sql("UPDATE group_states SET state_json=? WHERE group_did=?");
        sqlx::query(&sql)
            .bind(serde_json::to_string(&self.state).map_err(db_error)?)
            .bind(self.state.group_did.to_string())
            .execute(&mut *self.tx)
            .await
            .map_err(db_error)?;
        self.tx.commit().await.map_err(db_error)?;
        for record in &self.projections {
            crate::msg_center::MessageCenter::publish_box_changed_event(record, "upsert");
        }
        Ok(())
    }
}
