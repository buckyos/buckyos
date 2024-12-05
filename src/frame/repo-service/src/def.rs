use serde_json::Value;
use sqlx::FromRow;

pub const SERVICE_NAME: &str = "repo_service";
pub const REPO_CHUNK_MGR_ID: &str = "repo_chunk_mgr";
pub const INDEX_DIR_NAME: &str = "index";
pub const LOCAL_INDEX_DB: &str = "local.db";
pub const REPO_SOURCE_CONFIG_DB: &str = "source_config.db";
pub const TASK_EXPIRE_TIME: u64 = 30 * 60; //任务超时时间,单位秒

#[derive(Clone, Debug, FromRow)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub author: String, //author did
    pub chunk_id: String,
    pub dependencies: Value,
    pub sign: String, //sign of the chunk_id
    pub pub_time: i64,
}

pub struct SourceMeta {
    pub version: String,
    pub author: String,
    pub chunk_id: String,
    pub sign: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct SourceNodeConfig {
    pub id: i32,
    pub name: String,
    pub url: String,
    pub author: String,
    pub chunk_id: String,
    pub sign: String,
}
