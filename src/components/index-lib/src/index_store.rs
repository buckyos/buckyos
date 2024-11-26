use crate::PackageMeta;
use crate::Verifier;
use crate::{IndexError, IndexResult};
use log::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub struct IndexStore {
    db_path: String,
}

impl IndexStore {
    pub fn new(path: &str) -> Self {
        IndexStore {
            db_path: path.to_string(),
        }
    }

    pub fn open(&self) -> IndexResult<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS pkg_db (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                author TEXT NOT NULL,
                chunk_id TEXT NOT NULL,
                dependencies TEXT NOT NULL,
                sign TEXT NOT NULL,
                pub_time INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pkg_db_name ON pkg_db (name)",
            [],
        )?;
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_pkg_db_name_version ON pkg_db (name, version)",
            [],
        )?;
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_pkg_db_chunk_id ON pkg_db (chunk_id)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pkg_db_pub_time ON pkg_db (pub_time)",
            [],
        )?;
        Ok(conn)
    }

    pub async fn insert_pkg_meta(&self, pkg_meta: &PackageMeta) -> IndexResult<()> {
        let conn = self.open()?;
        match Verifier::verify(&pkg_meta.author, &pkg_meta.chunk_id, &pkg_meta.sign).await {
            Ok(_) => {
                conn.execute(
                    "INSERT INTO pkg_db (name, version, author, chunk_id, dependencies, sign, pub_time) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        pkg_meta.name,
                        pkg_meta.version,
                        pkg_meta.author,
                        pkg_meta.chunk_id,
                        pkg_meta.dependencies.to_string(),
                        pkg_meta.sign,
                        pkg_meta.pub_time
                    ],
                )?;
            }
            Err(e) => {
                error!("verify failed: {:?}", e);
                return Err(IndexError::VerifyError(e.to_string()));
            }
        }
        Ok(())
    }

    // version_desc: version or chunk_id
    pub fn remove_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
        is_desc_chunk_id: bool,
    ) -> IndexResult<()> {
        let conn = self.open()?;
        let sql = if is_desc_chunk_id {
            "DELETE FROM pkg_db WHERE name = ?1 AND chunk_id = ?2"
        } else {
            "DELETE FROM pkg_db WHERE name = ?1 AND version = ?2"
        };
        conn.execute(sql, params![name, version_desc])?;
        Ok(())
    }

    // version_desc: version or chunk_id
    pub fn get_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
        is_desc_chunk_id: bool,
    ) -> IndexResult<Option<PackageMeta>> {
        let conn = self.open()?;
        let mut stmt;
        if is_desc_chunk_id {
            stmt = conn.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 AND chunk_id = ?2")?;
        } else {
            stmt = conn.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 AND version = ?2")?;
        }
        let meta_info = stmt
            .query_row(params![name, version_desc], |row| {
                let dependencies_value: String = row.get(4)?;
                Ok(PackageMeta {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    author: row.get(2)?,
                    chunk_id: row.get(3)?,
                    dependencies: serde_json::from_str(&dependencies_value).map_err(|e| {
                        error!("serde_json::from_str failed: {:?}", e);
                        rusqlite::Error::InvalidQuery
                    })?,
                    sign: row.get(5)?,
                    pub_time: row.get(6)?,
                })
            })
            .optional()?;
        Ok(meta_info)
    }

    pub fn get_default_pkg_meta(&self, name: &str) -> IndexResult<Option<PackageMeta>> {
        let conn = self.open()?;
        // TODO: 精确的做法是选出所有，找到version最大的，暂时先以pub_time最大的为准
        let mut stmt = conn.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 ORDER BY pub_time DESC LIMIT 1")?;
        let meta_info = stmt
            .query_row(params![name], |row| {
                let dependencies_value: String = row.get(4)?;
                Ok(PackageMeta {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    author: row.get(2)?,
                    chunk_id: row.get(3)?,
                    dependencies: serde_json::from_str(&dependencies_value).map_err(|e| {
                        error!("serde_json::from_str failed: {:?}", e);
                        rusqlite::Error::InvalidQuery
                    })?,
                    sign: row.get(5)?,
                    pub_time: row.get(6)?,
                })
            })
            .optional()?;
        Ok(meta_info)
    }

    pub fn get_all_pkg_version(&self, name: &str) -> IndexResult<Vec<String>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT version FROM pkg_db WHERE name = ?1")?;
        let versions = stmt
            .query_map(params![name], |row| row.get(0))
            .unwrap()
            .map(|v| v.unwrap())
            .collect();
        Ok(versions)
    }
}
