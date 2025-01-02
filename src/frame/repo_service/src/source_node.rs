use crate::def::*;
use log::*;
use ndn_lib::ChunkId;
use serde_json::Value;
use sqlx::{Pool, Sqlite, SqlitePool};
use std::path::PathBuf;

fn is_valid_chunk_id(chunk_id: &str) -> bool {
    match ChunkId::new(chunk_id) {
        Ok(_) => true,
        Err(_) => false,
    }
}

#[derive(Debug, Clone)]
pub struct SourceNode {
    pub source_config: SourceNodeConfig,
    pub pool: SqlitePool,
}

impl SourceNode {
    pub async fn new(
        source_config: SourceNodeConfig,
        local_file: PathBuf,
        is_local: bool,
    ) -> RepoResult<Self> {
        let database_url = format!("sqlite://{}", local_file.to_string_lossy());
        let pool = SqlitePool::connect(&database_url).await?;

        if is_local {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS pkg_db (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL DEFAULT '',
                    version TEXT NOT NULL DEFAULT '',
                    author TEXT NOT NULL,
                    chunk_id TEXT NOT NULL DEFAULT '',
                    dependencies TEXT NOT NULL DEFAULT '',
                    sign TEXT NOT NULL DEFAULT '',
                    pub_time INTEGER NOT NULL DEFAULT 0
                )",
            )
            .execute(&pool)
            .await?;

            sqlx::query("CREATE INDEX IF NOT EXISTS idx_pkg_db_name ON pkg_db (name)")
                .execute(&pool)
                .await?;

            sqlx::query("CREATE INDEX IF NOT EXISTS idx_pkg_db_chunk_id ON pkg_db (chunk_id)")
                .execute(&pool)
                .await?;
        }
        Ok(SourceNode {
            source_config,
            pool,
        })
    }

    pub async fn insert_pkg_meta(&self, pkg_meta: &PackageMeta) -> RepoResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO pkg_db (name, version, author, chunk_id, dependencies, sign, pub_time) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&pkg_meta.name)
        .bind(&pkg_meta.version)
        .bind(&pkg_meta.author)
        .bind(&pkg_meta.chunk_id)
        .bind(&serde_json::to_string(&pkg_meta.dependencies)?)
        .bind(&pkg_meta.sign)
        .bind(&pkg_meta.pub_time)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_pkg_meta(&self, name: &str, version_desc: &str) -> RepoResult<()> {
        let sql = if is_valid_chunk_id(version_desc) {
            "DELETE FROM pkg_db WHERE name = ? AND chunk_id = ?"
        } else {
            "DELETE FROM pkg_db WHERE name = ? AND version = ?"
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(sql)
            .bind(name)
            .bind(version_desc)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
    ) -> RepoResult<Option<PackageMeta>> {
        let sql = if is_valid_chunk_id(version_desc) {
            "SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ? AND chunk_id = ?"
        } else {
            "SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ? AND version = ?"
        };
        let result = sqlx::query_as::<_, PackageMeta>(sql)
            .bind(name)
            .bind(version_desc)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result)
    }

    pub async fn get_default_pkg_meta(&self, name: &str) -> RepoResult<Option<PackageMeta>> {
        let result = sqlx::query_as::<_, PackageMeta>(
            "SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ? ORDER BY pub_time DESC LIMIT 1"
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    pub async fn get_all_pkg_version(&self, name: &str) -> RepoResult<Vec<String>> {
        let rows = sqlx::query_as::<_, PackageMeta>("SELECT version FROM pkg_db WHERE name = ?")
            .bind(name)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(|row| row.version).collect())
    }
}
