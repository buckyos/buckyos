use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub local_path: Option<String>,
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

#[async_trait]
pub trait RepoDb: Send + Sync {
    async fn upsert_object(&self, record: RepoObjectRecord) -> Result<()>;
    async fn get_object(&self, content_id: &str) -> Result<Option<RepoObjectRecord>>;
    async fn list_objects(&self) -> Result<Vec<RepoObjectRecord>>;
    async fn delete_object(&self, content_id: &str) -> Result<()>;

    async fn upsert_proof(&self, proof: RepoProofRecord) -> Result<()>;
    async fn list_proofs(&self, content_id: &str) -> Result<Vec<RepoProofRecord>>;

    async fn upsert_receipt(&self, receipt: RepoReceiptRecord) -> Result<()>;
    async fn stat(&self) -> Result<RepoDbStat>;
}

#[derive(Debug, Clone)]
pub struct SqliteRepoDb {
    db_path: PathBuf,
}

impl SqliteRepoDb {
    pub async fn open(db_path: PathBuf) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("create repo db dir failed: {}", parent.display())
            })?;
        }

        let db = Self { db_path };
        db.init().await?;
        Ok(db)
    }

    async fn init(&self) -> Result<()> {
        self.run_db(|conn| {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS objects (
                    content_id TEXT PRIMARY KEY,
                    content_name TEXT,
                    status TEXT NOT NULL DEFAULT 'collected',
                    origin TEXT NOT NULL,
                    meta TEXT NOT NULL,
                    owner_did TEXT,
                    author TEXT,
                    access_policy TEXT NOT NULL DEFAULT 'free',
                    price TEXT,
                    local_path TEXT,
                    content_size INTEGER,
                    collected_at INTEGER NOT NULL,
                    pinned_at INTEGER,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS proofs (
                    proof_id TEXT PRIMARY KEY,
                    content_id TEXT NOT NULL,
                    proof_kind TEXT NOT NULL,
                    action_type TEXT,
                    subject_id TEXT,
                    target_id TEXT,
                    base_on TEXT,
                    curator_did TEXT,
                    proof_data TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS receipts (
                    receipt_id TEXT PRIMARY KEY,
                    content_name TEXT NOT NULL,
                    buyer_did TEXT NOT NULL,
                    seller_did TEXT NOT NULL,
                    signature TEXT NOT NULL,
                    receipt_data TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_content_name ON objects(content_name);
                CREATE INDEX IF NOT EXISTS idx_status ON objects(status);
                CREATE INDEX IF NOT EXISTS idx_origin ON objects(origin);
                CREATE INDEX IF NOT EXISTS idx_owner_did ON objects(owner_did);
                CREATE INDEX IF NOT EXISTS idx_collected_at ON objects(collected_at);
                CREATE INDEX IF NOT EXISTS idx_proof_content_id ON proofs(content_id);
                CREATE INDEX IF NOT EXISTS idx_proof_kind ON proofs(proof_kind);
                CREATE INDEX IF NOT EXISTS idx_proof_action_type ON proofs(action_type);
                CREATE INDEX IF NOT EXISTS idx_proof_subject ON proofs(subject_id);
                CREATE INDEX IF NOT EXISTS idx_proof_target ON proofs(target_id);
                CREATE INDEX IF NOT EXISTS idx_proof_curator ON proofs(curator_did);
                CREATE INDEX IF NOT EXISTS idx_proof_created ON proofs(created_at);
                CREATE INDEX IF NOT EXISTS idx_proof_content_kind ON proofs(content_id, proof_kind);
                CREATE INDEX IF NOT EXISTS idx_receipt_content_name ON receipts(content_name);
                CREATE INDEX IF NOT EXISTS idx_receipt_buyer ON receipts(buyer_did);
                "#,
            )?;
            Ok(())
        })
        .await
    }

    async fn run_db<T, F>(&self, func: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let db_path = self.db_path.clone();
        tokio::task::spawn_blocking(move || -> Result<T> {
            let conn = open_connection(&db_path)?;
            func(&conn)
        })
        .await
        .map_err(|err| anyhow::anyhow!("repo db task failed: {err}"))?
    }
}

#[async_trait]
impl RepoDb for SqliteRepoDb {
    async fn upsert_object(&self, record: RepoObjectRecord) -> Result<()> {
        self.run_db(move |conn| {
            conn.execute(
                r#"
                INSERT INTO objects (
                    content_id, content_name, status, origin, meta, owner_did, author,
                    access_policy, price, local_path, content_size, collected_at, pinned_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                ON CONFLICT(content_id) DO UPDATE SET
                    content_name = excluded.content_name,
                    status = excluded.status,
                    origin = excluded.origin,
                    meta = excluded.meta,
                    owner_did = excluded.owner_did,
                    author = excluded.author,
                    access_policy = excluded.access_policy,
                    price = excluded.price,
                    local_path = excluded.local_path,
                    content_size = excluded.content_size,
                    collected_at = excluded.collected_at,
                    pinned_at = excluded.pinned_at,
                    updated_at = excluded.updated_at
                "#,
                params![
                    record.content_id,
                    record.content_name,
                    record.status,
                    record.origin,
                    serde_json::to_string(&record.meta)?,
                    record.owner_did,
                    record.author,
                    record.access_policy,
                    record.price,
                    record.local_path,
                    record.content_size,
                    record.collected_at,
                    record.pinned_at,
                    record.updated_at,
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn get_object(&self, content_id: &str) -> Result<Option<RepoObjectRecord>> {
        let content_id = content_id.to_string();
        self.run_db(move |conn| load_record_by_id(conn, &content_id)).await
    }

    async fn list_objects(&self) -> Result<Vec<RepoObjectRecord>> {
        self.run_db(load_all_records).await
    }

    async fn delete_object(&self, content_id: &str) -> Result<()> {
        let content_id = content_id.to_string();
        self.run_db(move |conn| {
            conn.execute("DELETE FROM objects WHERE content_id = ?1", params![content_id])?;
            Ok(())
        })
        .await
    }

    async fn upsert_proof(&self, proof: RepoProofRecord) -> Result<()> {
        self.run_db(move |conn| {
            conn.execute(
                r#"
                INSERT OR REPLACE INTO proofs (
                    proof_id, content_id, proof_kind, action_type, subject_id, target_id,
                    base_on, curator_did, proof_data, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
                params![
                    proof.proof_id,
                    proof.content_id,
                    proof.proof_kind,
                    proof.action_type,
                    proof.subject_id,
                    proof.target_id,
                    proof.base_on,
                    proof.curator_did,
                    proof.proof_data,
                    proof.created_at,
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn list_proofs(&self, content_id: &str) -> Result<Vec<RepoProofRecord>> {
        let content_id = content_id.to_string();
        self.run_db(move |conn| load_proof_rows(conn, &content_id)).await
    }

    async fn upsert_receipt(&self, receipt: RepoReceiptRecord) -> Result<()> {
        self.run_db(move |conn| {
            conn.execute(
                r#"
                INSERT OR REPLACE INTO receipts (
                    receipt_id, content_name, buyer_did, seller_did, signature, receipt_data, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    receipt.receipt_id,
                    receipt.content_name,
                    receipt.buyer_did,
                    receipt.seller_did,
                    receipt.signature,
                    serde_json::to_string(&receipt.receipt_data)?,
                    receipt.created_at,
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn stat(&self) -> Result<RepoDbStat> {
        self.run_db(move |conn| {
            Ok(RepoDbStat {
                total_objects: scalar_u64(conn, "SELECT COUNT(*) FROM objects")?,
                collected_objects: scalar_u64(
                    conn,
                    "SELECT COUNT(*) FROM objects WHERE status = 'collected'",
                )?,
                pinned_objects: scalar_u64(
                    conn,
                    "SELECT COUNT(*) FROM objects WHERE status = 'pinned'",
                )?,
                local_objects: scalar_u64(conn, "SELECT COUNT(*) FROM objects WHERE origin = 'local'")?,
                remote_objects: scalar_u64(
                    conn,
                    "SELECT COUNT(*) FROM objects WHERE origin = 'remote'",
                )?,
                total_content_bytes: scalar_u64(
                    conn,
                    "SELECT COALESCE(SUM(content_size), 0) FROM objects WHERE status = 'pinned'",
                )?,
                total_proofs: scalar_u64(conn, "SELECT COUNT(*) FROM proofs")?,
            })
        })
        .await
    }
}

fn open_connection(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("open sqlite db failed: {}", db_path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

fn scalar_u64(conn: &Connection, sql: &str) -> Result<u64> {
    let value = conn.query_row(sql, [], |row| row.get::<_, i64>(0))?;
    Ok(value.max(0) as u64)
}

fn load_record_by_id(conn: &Connection, content_id: &str) -> Result<Option<RepoObjectRecord>> {
    conn.query_row(
        r#"
        SELECT content_id, content_name, status, origin, meta, owner_did, author,
               access_policy, price, local_path, content_size, collected_at, pinned_at, updated_at
        FROM objects
        WHERE content_id = ?1
        "#,
        params![content_id],
        row_to_record,
    )
    .optional()
    .map_err(Into::into)
}

fn load_all_records(conn: &Connection) -> Result<Vec<RepoObjectRecord>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT content_id, content_name, status, origin, meta, owner_did, author,
               access_policy, price, local_path, content_size, collected_at, pinned_at, updated_at
        FROM objects
        ORDER BY updated_at DESC, collected_at DESC
        "#,
    )?;
    let rows = stmt.query_map([], row_to_record)?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RepoObjectRecord> {
    let meta_str: String = row.get(4)?;
    let meta = serde_json::from_str::<Value>(&meta_str).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(err),
        )
    })?;
    Ok(RepoObjectRecord {
        content_id: row.get(0)?,
        content_name: row.get(1)?,
        status: row.get(2)?,
        origin: row.get(3)?,
        meta,
        owner_did: row.get(5)?,
        author: row.get(6)?,
        access_policy: row.get(7)?,
        price: row.get(8)?,
        local_path: row.get(9)?,
        content_size: row.get::<_, Option<i64>>(10)?.map(|value| value.max(0) as u64),
        collected_at: row.get::<_, Option<i64>>(11)?.map(|value| value.max(0) as u64),
        pinned_at: row.get::<_, Option<i64>>(12)?.map(|value| value.max(0) as u64),
        updated_at: row.get::<_, Option<i64>>(13)?.map(|value| value.max(0) as u64),
    })
}

fn load_proof_rows(conn: &Connection, content_id: &str) -> Result<Vec<RepoProofRecord>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT proof_id, content_id, proof_kind, action_type, subject_id, target_id,
               base_on, curator_did, proof_data, created_at
        FROM proofs
        WHERE content_id = ?1
        ORDER BY created_at ASC
        "#,
    )?;
    let rows = stmt.query_map(params![content_id], |row| {
        Ok(RepoProofRecord {
            proof_id: row.get(0)?,
            content_id: row.get(1)?,
            proof_kind: row.get(2)?,
            action_type: row.get(3)?,
            subject_id: row.get(4)?,
            target_id: row.get(5)?,
            base_on: row.get(6)?,
            curator_did: row.get(7)?,
            proof_data: row.get(8)?,
            created_at: row.get::<_, i64>(9)?.max(0) as u64,
        })
    })?;
    let mut proofs = Vec::new();
    for row in rows {
        proofs.push(row?);
    }
    Ok(proofs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_repo_db_round_trip() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let db = SqliteRepoDb::open(dir.path().join("repo.db"))
            .await
            .expect("open sqlite repo db");

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
            local_path: Some("/tmp/demo".to_string()),
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

        let proofs = db
            .list_proofs("mix256:test")
            .await
            .expect("load proofs");
        assert_eq!(proofs.len(), 1);

        let stat = db.stat().await.expect("load stat");
        assert_eq!(stat.total_objects, 1);
        assert_eq!(stat.total_proofs, 1);
    }
}
