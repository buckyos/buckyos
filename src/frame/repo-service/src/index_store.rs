use crate::error::{RepoError, RepoResult};
use log::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub author: String, //author did
    pub chunk_id: String,
    pub dependencies: Value,
    pub sign: String, //sign of the chunk_id
    pub pub_time: u64,
}

pub trait IndexStore {
    fn insert_pkg_meta(&self, pkg_meta: &PackageMeta) -> RepoResult<()>;
    fn remove_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
        is_desc_chunk_id: bool,
    ) -> RepoResult<()>;
    fn get_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
        is_desc_chunk_id: bool,
    ) -> RepoResult<Option<PackageMeta>>;
    fn get_default_pkg_meta(&self, name: &str) -> RepoResult<Option<PackageMeta>>;
    fn get_all_pkg_version(&self, name: &str) -> RepoResult<Vec<String>>;
}

impl IndexStore for Connection {
    fn insert_pkg_meta(&self, pkg_meta: &PackageMeta) -> RepoResult<()> {
        //如果需要事务，在外部控制
        self.execute(
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
        Ok(())
    }

    // version_desc: version or chunk_id
    fn remove_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
        is_desc_chunk_id: bool,
    ) -> RepoResult<()> {
        let sql = if is_desc_chunk_id {
            "DELETE FROM pkg_db WHERE name = ?1 AND chunk_id = ?2"
        } else {
            "DELETE FROM pkg_db WHERE name = ?1 AND version = ?2"
        };
        self.execute(sql, params![name, version_desc])?;
        Ok(())
    }

    // version_desc: version or chunk_id
    fn get_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
        is_desc_chunk_id: bool,
    ) -> RepoResult<Option<PackageMeta>> {
        let mut stmt;
        if is_desc_chunk_id {
            stmt = self.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 AND chunk_id = ?2")?;
        } else {
            stmt = self.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 AND version = ?2")?;
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

    fn get_default_pkg_meta(&self, name: &str) -> RepoResult<Option<PackageMeta>> {
        // TODO: 精确的做法是选出所有，找到version最大的，暂时先以pub_time最大的为准
        let mut stmt = self.prepare("SELECT name, version, author, chunk_id, dependencies, sign, pub_time FROM pkg_db WHERE name = ?1 ORDER BY pub_time DESC LIMIT 1")?;
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

    fn get_all_pkg_version(&self, name: &str) -> RepoResult<Vec<String>> {
        let mut stmt = self.prepare("SELECT version FROM pkg_db WHERE name = ?1")?;
        let versions = stmt
            .query_map(params![name], |row| row.get(0))
            .unwrap()
            .map(|v| v.unwrap())
            .collect();
        Ok(versions)
    }
}
