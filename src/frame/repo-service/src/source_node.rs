use crate::error::{RepoError, RepoResult};
use crate::index_store::*;
use crate::source_manager::SourceNodeConfig;
use rusqlite::Connection;
use std::path::PathBuf;

pub struct SourceNode {
    pub source_config: SourceNodeConfig,
    pub conn: Connection,
}

impl SourceNode {
    pub fn new(source_config: SourceNodeConfig) -> RepoResult<Self> {
        let conn = Connection::open(source_config.local_file.clone())?;
        Ok(SourceNode {
            source_config,
            conn,
        })
    }

    pub async fn insert_pkg_meta(&self, pkg_meta: &PackageMeta) -> RepoResult<()> {
        self.conn.insert_pkg_meta(pkg_meta)
    }

    pub async fn remove_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
        is_desc_chunk_id: bool,
    ) -> RepoResult<()> {
        self.conn
            .remove_pkg_meta(name, version_desc, is_desc_chunk_id)
    }

    pub async fn get_pkg_meta(
        &self,
        name: &str,
        version_desc: &str,
        is_desc_chunk_id: bool,
    ) -> RepoResult<Option<PackageMeta>> {
        self.conn.get_pkg_meta(name, version_desc, is_desc_chunk_id)
    }

    pub async fn get_default_pkg_meta(&self, name: &str) -> RepoResult<Option<PackageMeta>> {
        self.conn.get_default_pkg_meta(name)
    }
}
