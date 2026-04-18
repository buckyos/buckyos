use buckyos_api::{
    get_rdb_instance, RdbBackend, REPO_SERVICE_RDB_INSTANCE_ID, REPO_SERVICE_RDB_SCHEMA_POSTGRES,
    REPO_SERVICE_RDB_SCHEMA_SQLITE, REPO_SERVICE_SERVICE_NAME,
};
use log::info;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};
use std::sync::Once;

static INSTALL_DRIVERS: Once = Once::new();

fn ensure_any_drivers_installed() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

#[derive(Debug, Clone)]
pub struct RepoObjectRecord {
    pub content_id: String,
    pub content_name: Option<String>,
    pub status: String,
    pub origin: String,
    pub meta: Value,
    pub owner_did: Option<String>,
    pub author: Option<String>,
    pub access_policy: String,
    pub price: Option<String>,
    pub content_size: Option<u64>,
    pub collected_at: Option<u64>,
    pub pinned_at: Option<u64>,
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RepoProofRecord {
    pub proof_id: String,
    pub content_id: String,
    pub proof_kind: String,
    pub action_type: Option<String>,
    pub subject_id: Option<String>,
    pub target_id: Option<String>,
    pub base_on: Option<String>,
    pub curator_did: Option<String>,
    pub proof_data: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoReceiptRecord {
    pub receipt_id: String,
    pub content_name: String,
    pub buyer_did: String,
    pub seller_did: String,
    pub signature: String,
    pub receipt_data: Value,
    pub created_at: u64,
}

#[derive(Debug, Clone, Default)]
pub struct RepoDbStat {
    pub total_objects: u64,
    pub collected_objects: u64,
    pub pinned_objects: u64,
    pub local_objects: u64,
    pub remote_objects: u64,
    pub total_content_bytes: u64,
    pub total_proofs: u64,
}

/// Handle to the repo-service rdb. Wraps an `sqlx::AnyPool` — the pool itself
/// is already `Send + Sync + Clone` (internally `Arc`-backed) and manages its
/// own per-connection locking, so a `RepoDb` is safe to share via `Arc<RepoDb>`
/// with no outer Rust-level lock.
pub struct RepoDb {
    pool: AnyPool,
    backend: RdbBackend,
}

pub type DbResult<T> = Result<T, sqlx::Error>;

impl RepoDb {
    /// Open a pool against `connection`. `schema` is the DDL to apply (usually
    /// what the service spec carried for the chosen backend); an empty / None
    /// value means "use the compile-time default for `backend`".
    pub async fn open(
        connection: &str,
        backend: RdbBackend,
        schema: Option<&str>,
    ) -> Result<Self, String> {
        ensure_any_drivers_installed();
        let opts = AnyPoolOptions::new().max_connections(8);
        let pool = opts
            .connect(connection)
            .await
            .map_err(|err| format!("open repo-service db at {}: {}", connection, err))?;
        let db = RepoDb { pool, backend };
        db.apply_schema(schema)
            .await
            .map_err(|err| format!("apply repo-service schema: {}", err))?;
        Ok(db)
    }

    /// Resolve the repo-service rdb instance from the service spec and open a
    /// pool against it. This is the production entry point.
    pub async fn open_from_service_spec() -> Result<Self, String> {
        let instance = get_rdb_instance(
            REPO_SERVICE_SERVICE_NAME,
            None,
            REPO_SERVICE_RDB_INSTANCE_ID,
        )
        .await
        .map_err(|err| format!("resolve repo-service rdb instance failed: {}", err))?;
        info!("repo_db.open {}", instance.connection);
        Self::open(
            &instance.connection,
            instance.backend,
            instance.schema.as_deref(),
        )
        .await
    }

    async fn apply_schema(&self, override_ddl: Option<&str>) -> DbResult<()> {
        let ddl: &str =
            override_ddl
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(match self.backend {
                    RdbBackend::Sqlite => REPO_SERVICE_RDB_SCHEMA_SQLITE,
                    RdbBackend::Postgres => REPO_SERVICE_RDB_SCHEMA_POSTGRES,
                });
        // sqlx::AnyPool::execute accepts a single statement at a time when the
        // backend driver is strict, so split on ';' and run each non-empty
        // fragment.
        for statement in split_sql_statements(ddl) {
            self.pool.execute(statement.as_str()).await?;
        }
        Ok(())
    }

    fn pool(&self) -> &AnyPool {
        &self.pool
    }

    pub async fn upsert_object(&self, record: RepoObjectRecord) -> DbResult<()> {
        let meta_str = serde_json::to_string(&record.meta).unwrap_or_else(|_| "null".to_string());
        let sql = self.render_sql(
            "INSERT INTO objects (
                content_id, content_name, status, origin, meta, owner_did, author,
                access_policy, price, content_size, collected_at, pinned_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(content_id) DO UPDATE SET
                content_name = excluded.content_name,
                status = excluded.status,
                origin = excluded.origin,
                meta = excluded.meta,
                owner_did = excluded.owner_did,
                author = excluded.author,
                access_policy = excluded.access_policy,
                price = excluded.price,
                content_size = excluded.content_size,
                collected_at = excluded.collected_at,
                pinned_at = excluded.pinned_at,
                updated_at = excluded.updated_at",
        );
        sqlx::query(&sql)
            .bind(record.content_id)
            .bind(record.content_name)
            .bind(record.status)
            .bind(record.origin)
            .bind(meta_str)
            .bind(record.owner_did)
            .bind(record.author)
            .bind(record.access_policy)
            .bind(record.price)
            .bind(u64_to_i64_opt(record.content_size))
            .bind(u64_to_i64_opt(record.collected_at))
            .bind(u64_to_i64_opt(record.pinned_at))
            .bind(u64_to_i64_opt(record.updated_at))
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn get_object(&self, content_id: &str) -> DbResult<Option<RepoObjectRecord>> {
        let sql = self.render_sql(
            "SELECT content_id, content_name, status, origin, meta, owner_did, author,
                    access_policy, price, content_size, collected_at, pinned_at, updated_at
             FROM objects
             WHERE content_id = ?",
        );
        let row = sqlx::query(&sql)
            .bind(content_id.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(object_from_row).transpose()
    }

    pub async fn list_objects(&self) -> DbResult<Vec<RepoObjectRecord>> {
        let sql = self.render_sql(
            "SELECT content_id, content_name, status, origin, meta, owner_did, author,
                    access_policy, price, content_size, collected_at, pinned_at, updated_at
             FROM objects
             ORDER BY updated_at DESC, collected_at DESC",
        );
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        rows.into_iter().map(object_from_row).collect()
    }

    pub async fn delete_object(&self, content_id: &str) -> DbResult<()> {
        let sql = self.render_sql("DELETE FROM objects WHERE content_id = ?");
        sqlx::query(&sql)
            .bind(content_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn upsert_proof(&self, proof: RepoProofRecord) -> DbResult<()> {
        let sql = self.render_sql(
            "INSERT INTO proofs (
                proof_id, content_id, proof_kind, action_type, subject_id, target_id,
                base_on, curator_did, proof_data, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(proof_id) DO UPDATE SET
                content_id = excluded.content_id,
                proof_kind = excluded.proof_kind,
                action_type = excluded.action_type,
                subject_id = excluded.subject_id,
                target_id = excluded.target_id,
                base_on = excluded.base_on,
                curator_did = excluded.curator_did,
                proof_data = excluded.proof_data,
                created_at = excluded.created_at",
        );
        sqlx::query(&sql)
            .bind(proof.proof_id)
            .bind(proof.content_id)
            .bind(proof.proof_kind)
            .bind(proof.action_type)
            .bind(proof.subject_id)
            .bind(proof.target_id)
            .bind(proof.base_on)
            .bind(proof.curator_did)
            .bind(proof.proof_data)
            .bind(proof.created_at as i64)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn list_proofs(&self, content_id: &str) -> DbResult<Vec<RepoProofRecord>> {
        let sql = self.render_sql(
            "SELECT proof_id, content_id, proof_kind, action_type, subject_id, target_id,
                    base_on, curator_did, proof_data, created_at
             FROM proofs
             WHERE content_id = ?
             ORDER BY created_at ASC",
        );
        let rows = sqlx::query(&sql)
            .bind(content_id.to_string())
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(proof_from_row).collect()
    }

    pub async fn upsert_receipt(&self, receipt: RepoReceiptRecord) -> DbResult<()> {
        let receipt_data_str =
            serde_json::to_string(&receipt.receipt_data).unwrap_or_else(|_| "null".to_string());
        let sql = self.render_sql(
            "INSERT INTO receipts (
                receipt_id, content_name, buyer_did, seller_did, signature, receipt_data, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(receipt_id) DO UPDATE SET
                content_name = excluded.content_name,
                buyer_did = excluded.buyer_did,
                seller_did = excluded.seller_did,
                signature = excluded.signature,
                receipt_data = excluded.receipt_data,
                created_at = excluded.created_at",
        );
        sqlx::query(&sql)
            .bind(receipt.receipt_id)
            .bind(receipt.content_name)
            .bind(receipt.buyer_did)
            .bind(receipt.seller_did)
            .bind(receipt.signature)
            .bind(receipt_data_str)
            .bind(receipt.created_at as i64)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn stat(&self) -> DbResult<RepoDbStat> {
        Ok(RepoDbStat {
            total_objects: self.scalar_u64("SELECT COUNT(*) FROM objects").await?,
            collected_objects: self
                .scalar_u64("SELECT COUNT(*) FROM objects WHERE status = 'collected'")
                .await?,
            pinned_objects: self
                .scalar_u64("SELECT COUNT(*) FROM objects WHERE status = 'pinned'")
                .await?,
            local_objects: self
                .scalar_u64("SELECT COUNT(*) FROM objects WHERE origin = 'local'")
                .await?,
            remote_objects: self
                .scalar_u64("SELECT COUNT(*) FROM objects WHERE origin = 'remote'")
                .await?,
            total_content_bytes: self
                .scalar_u64(
                    "SELECT COALESCE(SUM(content_size), 0) FROM objects WHERE status = 'pinned'",
                )
                .await?,
            total_proofs: self.scalar_u64("SELECT COUNT(*) FROM proofs").await?,
        })
    }

    async fn scalar_u64(&self, sql: &str) -> DbResult<u64> {
        let row = sqlx::query(sql).fetch_one(self.pool()).await?;
        let value: i64 = row.try_get(0)?;
        Ok(value.max(0) as u64)
    }

    /// Translate `?` placeholders into `$N` form for postgres. Other backends
    /// pass through unchanged.
    fn render_sql(&self, sql: &str) -> String {
        match self.backend {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }
}

fn u64_to_i64_opt(value: Option<u64>) -> Option<i64> {
    value.map(|v| v as i64)
}

fn i64_opt_to_u64(value: Option<i64>) -> Option<u64> {
    value.map(|v| v.max(0) as u64)
}

fn object_from_row(row: AnyRow) -> DbResult<RepoObjectRecord> {
    let meta_str: String = row.try_get("meta")?;
    let meta = serde_json::from_str::<Value>(&meta_str).unwrap_or(Value::Null);
    Ok(RepoObjectRecord {
        content_id: row.try_get("content_id")?,
        content_name: row.try_get("content_name")?,
        status: row.try_get("status")?,
        origin: row.try_get("origin")?,
        meta,
        owner_did: row.try_get("owner_did")?,
        author: row.try_get("author")?,
        access_policy: row.try_get("access_policy")?,
        price: row.try_get("price")?,
        content_size: i64_opt_to_u64(row.try_get("content_size")?),
        collected_at: i64_opt_to_u64(row.try_get("collected_at")?),
        pinned_at: i64_opt_to_u64(row.try_get("pinned_at")?),
        updated_at: i64_opt_to_u64(row.try_get("updated_at")?),
    })
}

fn proof_from_row(row: AnyRow) -> DbResult<RepoProofRecord> {
    Ok(RepoProofRecord {
        proof_id: row.try_get("proof_id")?,
        content_id: row.try_get("content_id")?,
        proof_kind: row.try_get("proof_kind")?,
        action_type: row.try_get("action_type")?,
        subject_id: row.try_get("subject_id")?,
        target_id: row.try_get("target_id")?,
        base_on: row.try_get("base_on")?,
        curator_did: row.try_get("curator_did")?,
        proof_data: row.try_get("proof_data")?,
        created_at: row.try_get::<i64, _>("created_at")?.max(0) as u64,
    })
}

fn rewrite_placeholders_to_dollar(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut idx = 0u32;
    let mut in_single = false;
    let mut in_double = false;
    for ch in sql.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(ch);
            }
            '?' if !in_single && !in_double => {
                idx += 1;
                out.push('$');
                out.push_str(&idx.to_string());
            }
            _ => out.push(ch),
        }
    }
    out
}

fn split_sql_statements(ddl: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut buf = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for ch in ddl.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                buf.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                buf.push(ch);
            }
            ';' if !in_single && !in_double => {
                let trimmed = buf.trim();
                if !trimmed.is_empty() {
                    stmts.push(trimmed.to_string());
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    let trimmed = buf.trim();
    if !trimmed.is_empty() {
        stmts.push(trimmed.to_string());
    }
    stmts
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn open_test_db() -> (RepoDb, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("repo.db");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
        let db = RepoDb::open(&conn, RdbBackend::Sqlite, None)
            .await
            .expect("open sqlite repo db");
        (db, dir)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sqlite_repo_db_round_trip() {
        let (db, _dir) = open_test_db().await;

        db.upsert_object(RepoObjectRecord {
            content_id: "mix256:test".to_string(),
            content_name: Some("demo".to_string()),
            status: "pinned".to_string(),
            origin: "local".to_string(),
            meta: serde_json::json!({"content":"mix256:test"}),
            owner_did: Some("did:bns:alice".to_string()),
            author: Some("alice".to_string()),
            access_policy: "free".to_string(),
            price: None,
            content_size: Some(12),
            collected_at: Some(1),
            pinned_at: Some(2),
            updated_at: Some(3),
        })
        .await
        .expect("upsert object");

        db.upsert_proof(RepoProofRecord {
            proof_id: "proof-1".to_string(),
            content_id: "mix256:test".to_string(),
            proof_kind: "action".to_string(),
            action_type: Some("download".to_string()),
            subject_id: Some("actor:alice".to_string()),
            target_id: Some("mix256:test".to_string()),
            base_on: None,
            curator_did: None,
            proof_data: "{}".to_string(),
            created_at: 10,
        })
        .await
        .expect("upsert proof");

        let object = db
            .get_object("mix256:test")
            .await
            .expect("load object")
            .expect("object exists");
        assert_eq!(object.content_name.as_deref(), Some("demo"));

        let proofs = db.list_proofs("mix256:test").await.expect("load proofs");
        assert_eq!(proofs.len(), 1);

        let stat = db.stat().await.expect("load stat");
        assert_eq!(stat.total_objects, 1);
        assert_eq!(stat.total_proofs, 1);
    }

    #[test]
    fn rewrite_placeholders_handles_quotes() {
        assert_eq!(
            rewrite_placeholders_to_dollar("SELECT ? FROM t WHERE s = '?' AND x = ?"),
            "SELECT $1 FROM t WHERE s = '?' AND x = $2"
        );
    }
}
