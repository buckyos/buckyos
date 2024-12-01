use crate::downloader::*;
use crate::error::*;
use crate::source_node::*;
use crate::verifier::*;
use rusqlite::Connection;
use std::fmt::format;
use std::path::PathBuf;
use tokio::sync::RwLock;

const REPO_CHUNK_MGR_ID: &str = "repo_chunk_mgr";

pub struct SourceMeta {
    pub version: String,
    pub author: String,
    pub chunk_id: String,
    pub sign: String,
}

#[derive(Debug, Clone)]
pub struct SourceNodeConfig {
    pub id: i32,
    pub name: String,
    pub url: String,
    pub author: String,
    pub chunk_id: String,
    pub sign: String,
    pub local_file: String,
}

pub struct SourceManager {
    pub source_list: RwLock<Vec<SourceNode>>,
    pub source_config_db_path: PathBuf,
}

impl SourceManager {
    pub fn new(config_db_path: &PathBuf) -> Self {
        Self {
            source_list: RwLock::new(Vec::new()),
            source_config_db_path: config_db_path.clone(),
        }
    }

    fn get_source_config_list(&self) -> RepoResult<Vec<SourceNodeConfig>> {
        let conn = Connection::open(&self.source_config_db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS source_node (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL DEFAULT '',
                author TEXT NOT NULL,
                chunk_id TEXT NOT NULL DEFAULT '',
                sign TEXT NOT NULL DEFAULT '',
                local_file TEXT NOT NULL DEFAULT ''
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_source_node_name ON source_node (name)",
            [],
        )?;
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_source_node_chunk_id, ON source_node (chunk_id)",
            [],
        )?;
        let mut stmt = conn
            .prepare("SELECT id, name, url, author, chunk_id, sign, local_file FROM source_node")?;
        let source_iter = stmt.query_map([], |row| {
            Ok(SourceNodeConfig {
                id: row.get(0)?,
                name: row.get(1)?,
                url: row.get(2)?,
                author: row.get(3)?,
                chunk_id: row.get(4)?,
                sign: row.get(5)?,
                local_file: row.get(6)?,
            })
        })?;
        let mut source_config_list = Vec::new();
        for source in source_iter {
            source_config_list.push(source?);
        }
        Ok(source_config_list)
    }

    async fn make_sure_source_file_exists(
        url: &str,
        author: &str,
        chunk_id: &str,
        sign: &str,
        local_file: &PathBuf,
    ) -> RepoResult<()> {
        if !local_file.exists() {
            //从source.url请求对应的chunk_id
            pull_remote_chunk(url, author, sign, chunk_id, REPO_CHUNK_MGR_ID).await?;
            chunk_to_local_file(&chunk_id, REPO_CHUNK_MGR_ID, &local_file).await?;
        }
        Ok(())
    }

    async fn update_all_source(&self) -> RepoResult<()> {
        let source_config_list = self.get_source_config_list()?;
        let mut new_source_list = Vec::new();
        for mut source_config in source_config_list {
            //通过url请求最新的source_meta
            if source_config.url.is_empty() {
                //本地的source不会有url，所以不会变化
                continue;
            }
            let source_meta = get_remote_source_meta(&source_config.url).await?;
            if source_meta.chunk_id != source_config.chunk_id {
                //source有更新，需要重新下载
                let local_file = PathBuf::from(format!(
                    "{}_{}.db",
                    source_config.name, source_meta.chunk_id
                ));
                Self::make_sure_source_file_exists(
                    &source_config.url,
                    &source_config.author,
                    &source_meta.chunk_id,
                    &source_meta.sign,
                    &local_file,
                )
                .await?;
                //更新source_config
                source_config.chunk_id = source_meta.chunk_id;
                source_config.sign = source_meta.sign;
                source_config.local_file = local_file.to_str().unwrap().to_string();
            }
            let source_node = SourceNode::new(source_config)?;
            new_source_list.push(source_node);
        }

        let mut source_list = self.source_list.write().await;
        *source_list = new_source_list;

        Ok(())
    }

    pub async fn build_source_list(&self) -> RepoResult<()> {
        Ok(())
    }
}
