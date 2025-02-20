use crate::def::*;
use log::*;
use ndn_lib::ChunkId;
use package_lib::*;
use serde_json::Value;
use sqlx::{database, error, Pool, Sqlite, SqlitePool};
use std::collections::HashMap;
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
        let database_url = if is_local {
            format!("sqlite://{}?mode=rwc", local_file.to_string_lossy())
        } else {
            format!("sqlite://{}", local_file.to_string_lossy())
        };
        let pool = SqlitePool::connect(&database_url).await?;

        if is_local {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS pkg_db (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    pkg_name TEXT NOT NULL,
                    version TEXT NOT NULL,
                    category TEXT NOT NULL,
                    hostname TEXT NOT NULL,
                    chunk_id TEXT DEFAULT NULL,
                    dependencies TEXT NOT NULL DEFAULT '',
                    jwt TEXT NOT NULL,
                    pub_time INTEGER NOT NULL DEFAULT 0,
                    UNIQUE(pkg_name, version)
                )",
            )
            .execute(&pool)
            .await?;

            sqlx::query("CREATE INDEX IF NOT EXISTS idx_pkg_db_pkg_name ON pkg_db (pkg_name)")
                .execute(&pool)
                .await?;

            sqlx::query("CREATE INDEX IF NOT EXISTS idx_pkg_db_chunk_id ON pkg_db (chunk_id)")
                .execute(&pool)
                .await?;

            //pkg_name与version创建唯一索引
            sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_pkg_db_pkg_name_version ON pkg_db (pkg_name, version)")
                .execute(&pool)
                .await?;
        }
        Ok(SourceNode {
            source_config,
            pool,
        })
    }

    pub async fn insert_pkg_meta(&self, pkg_meta: &PackageMeta) -> RepoResult<()> {
        // 判断dependencies是不是一个合法的json
        let _: HashMap<String, String> =
            serde_json::from_str(&pkg_meta.dependencies).map_err(|err| {
                error!("parse dependencies failed, err:{}", err);
                RepoError::JsonError(err)
            })?;

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO pkg_db (pkg_name, version, category, hostname, chunk_id, dependencies, jwt, pub_time) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&pkg_meta.pkg_name)
        .bind(&pkg_meta.version)
        .bind(&pkg_meta.category)
        .bind(&pkg_meta.hostname)
        .bind(&pkg_meta.chunk_id)
        .bind(&pkg_meta.dependencies)
        .bind(&pkg_meta.jwt)
        .bind(&pkg_meta.pub_time)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        info!(
            "insert pkg meta success, pkg_name:{}, version:{}, category:{}, hostname:{}, chunk_id:{:?}, dependencies:{}, jwt:{}, pub_time:{}",
            pkg_meta.pkg_name,
            pkg_meta.version,
            pkg_meta.category,
            pkg_meta.hostname,
            pkg_meta.chunk_id,
            pkg_meta.dependencies,
            pkg_meta.jwt,
            pkg_meta.pub_time
        );

        Ok(())
    }

    pub async fn remove_pkg_meta(&self, pkg_name: &str, version_desc: &str) -> RepoResult<()> {
        let sql = if is_valid_chunk_id(version_desc) {
            "DELETE FROM pkg_db WHERE pkg_name = ? AND chunk_id = ?"
        } else {
            "DELETE FROM pkg_db WHERE pkg_name = ? AND version = ?"
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(sql)
            .bind(pkg_name)
            .bind(version_desc)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_pkg_meta(
        &self,
        pkg_name: &str,
        version_desc: &str,
    ) -> RepoResult<Option<PackageMeta>> {
        //如果不是>或者<开头，那么就是一个具体的版本号
        if !version_desc.starts_with('>')
            && !version_desc.starts_with('<')
            && !version_desc.starts_with('=')
        {
            return self.get_exact_pkg_meta(pkg_name, version_desc).await;
        }

        //获取所有的版本号
        let all_versions = self.get_all_pkg_version(pkg_name).await?;
        let matched_version = VersionUtil::find_matched_version(version_desc, &all_versions)
            .map_err(|err| {
                let err_msg = format!(
                    "find_matched_version failed, pkg-name:{}, target version:{}, err:{}",
                    pkg_name, version_desc, err
                );
                error!("{}", err_msg);
                RepoError::VersionNotFoundError(err_msg)
            })?;

        self.get_exact_pkg_meta(pkg_name, &matched_version).await
    }

    // version is already a specific version number or * or a chunk_id
    pub async fn get_exact_pkg_meta(
        &self,
        pkg_name: &str,
        version: &str,
    ) -> RepoResult<Option<PackageMeta>> {
        let sql = if is_valid_chunk_id(version) {
            "SELECT * FROM pkg_db WHERE pkg_name = ? AND chunk_id = ?"
        } else if version == "*" {
            "SELECT * FROM pkg_db WHERE pkg_name = ? ORDER BY pub_time DESC LIMIT 1"
        } else {
            //如果第一个字符是>或者<，return error
            if version.starts_with('>') || version.starts_with('<') || version.starts_with('=') {
                error!(
                    "Invalid version expression, should be exact version number: {}",
                    version
                );
                return Err(RepoError::VersionError(version.to_string()));
            }
            "SELECT * FROM pkg_db WHERE pkg_name = ? AND version = ?"
        };

        let result = sqlx::query_as::<_, PackageMeta>(sql)
            .bind(pkg_name)
            .bind(version)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result)
    }

    pub async fn get_default_pkg_meta(&self, pkg_name: &str) -> RepoResult<Option<PackageMeta>> {
        let result = sqlx::query_as::<_, PackageMeta>(
            "SELECT * FROM pkg_db WHERE pkg_name = ? ORDER BY pub_time DESC LIMIT 1",
        )
        .bind(pkg_name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    pub async fn get_all_pkg_version(&self, pkg_name: &str) -> RepoResult<Vec<String>> {
        let rows = sqlx::query_as::<_, (String,)>("SELECT version FROM pkg_db WHERE pkg_name = ?")
            .bind(pkg_name)
            .fetch_all(&self.pool)
            .await?;

        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.0);
        }

        Ok(versions)
    }

    pub async fn get_all_latest_pkg(&self, category: Option<&str>) -> RepoResult<Vec<PackageMeta>> {
        let rows= match category {
            Some(category) => {
                sqlx::query_as::<_, PackageMeta>(
                    "SELECT * FROM pkg_db WHERE category = ? AND (pkg_name, pub_time) IN (SELECT pkg_name, MAX(pub_time) FROM pkg_db GROUP BY pkg_name)"
                )
                .bind(category)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, PackageMeta>(
                    "SELECT * FROM pkg_db WHERE (pkg_name, pub_time) IN (SELECT pkg_name, MAX(pub_time) FROM pkg_db GROUP BY pkg_name)"
                )
                .fetch_all(&self.pool)
                .await?
            }
        };

        debug!(
            "get_all_latest_pkg, index:{}, category:{:?} count:{:?}",
            self.source_config.hostname,
            category,
            rows.len()
        );

        Ok(rows)
    }
}
