use crate::error::{RepoError, RepoResult};
//use crate::index_store::*;
use crate::def::*;
use log::*;
use ndn_lib::ChunkId;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::PathBuf;

fn is_valid_chunk_id(chunk_id: &str) -> bool {
    match ChunkId::new(chunk_id) {
        Ok(_) => true,
        Err(_) => false,
    }
}

pub struct SourceNode {
    pub source_config: SourceNodeConfig,
    pub conn: Connection,
}

impl SourceNode {
    pub fn new(
        source_config: SourceNodeConfig,
        local_file: PathBuf,
        is_local: bool,
    ) -> RepoResult<Self> {
        let conn = Connection::open(local_file)?;
        if is_local {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS pkg_db (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL DEFAULT '',
                    version TEXT NOT NULL DEFAULT '',
                    author TEXT NOT NULL,
                    chunk_id TEXT NOT NULL DEFAULT '',
                    dependencies TEXT NOT NULL DEFAULT '',
                    sign TEXT NOT NULL DEFAULT '',
                    pub_time INTEGER NOT NULL
                )",
                [],
            )?;
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_pkg_db_name ON pkg_db (name)",
                [],
            )?;
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_pkg_db_chunk_id ON pkg_db (chunk_id)",
                [],
            )?;
        }
        Ok(SourceNode {
            source_config,
            conn,
        })
    }

    pub fn insert_pkg_meta(&mut self, pkg_meta: &PackageMeta) -> RepoResult<()> {
        //self.conn.insert_pkg_meta(pkg_meta)
        let tx = self.conn.transaction()?;
        tx.execute(
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
        tx.commit()?;
        Ok(())
    }

    pub fn remove_pkg_meta(&mut self, name: &str, version_desc: &str) -> RepoResult<()> {
        let sql = if is_valid_chunk_id(version_desc) {
            "DELETE FROM pkg_db WHERE name = ?1 AND chunk_id = ?2"
        } else {
            "DELETE FROM pkg_db WHERE name = ?1 AND version = ?2"
        };
        let tx = self.conn.transaction()?;
        tx.execute(sql, params![name, version_desc])?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_pkg_meta(&self, name: &str, version_desc: &str) -> RepoResult<Option<PackageMeta>> {
        let mut stmt;
        if is_valid_chunk_id(version_desc) {
            stmt = self.conn.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 AND chunk_id = ?2")?;
        } else {
            stmt = self.conn.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 AND version = ?2")?;
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

    pub fn get_default_pkg_meta(&self, name: &str) -> RepoResult<Option<PackageMeta>> {
        let mut stmt = self.conn.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 ORDER BY pub_time DESC LIMIT 1")?;
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

    pub fn get_all_pkg_version(&self, name: &str) -> RepoResult<Vec<String>> {
        //self.conn.get_all_pkg_version(name)
        let mut stmt = self
            .conn
            .prepare("SELECT version FROM pkg_db WHERE name = ?1")?;
        let version_iter = stmt.query_map(params![name], |row| Ok(row.get(0)?))?;
        let mut version_list = Vec::new();
        for version in version_iter {
            version_list.push(version?);
        }
        Ok(version_list)
    }
}
