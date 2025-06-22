use crate::{
    ChunkHasher, ChunkId, ChunkReader, ChunkWriter, LinkData, NdnError, NdnResult, ObjId,
    ObjectLink,
};
use async_trait::async_trait;
use rusqlite::types::{ToSql, FromSql, ValueRef};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncSeek};

pub enum ObjectState {
    Exist,
    NotCompleted,
    NotExist,
    Object(String),           //json_str
    Reader(ChunkReader, u64), //u64 is the chunk size
    Link(LinkData),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChunkState {
    New,         //刚创建
    Completed,   //完成
    Incompleted, //未完成
    Disabled,    //禁用
    NotExist,    //不存在
    Link(LinkData),
}

impl ChunkState {
    pub fn from_str(s: &str) -> Self {
        match s {
            "new" => ChunkState::New,
            "completed" => ChunkState::Completed,
            "incompleted" => ChunkState::Incompleted,
            "disabled" => ChunkState::Disabled,
            "not_exist" => ChunkState::NotExist,
            _ => ChunkState::NotExist,
        }
    }

    pub fn to_str(&self) -> String {
        match self {
            ChunkState::New => "new".to_string(),
            ChunkState::Completed => "completed".to_string(),
            ChunkState::Incompleted => "incompleted".to_string(),
            ChunkState::Disabled => "disabled".to_string(),
            ChunkState::NotExist => "not_exist".to_string(),
            ChunkState::Link(link_data) => link_data.to_string(),
        }
    }
}

impl ToSql for ChunkState {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let s = match self {
            ChunkState::New => "new",
            ChunkState::Completed => "completed",
            ChunkState::Incompleted => "incompleted",
            ChunkState::Disabled => "disabled",
            ChunkState::NotExist => "not_exist",
            ChunkState::Link(_) => panic!("ChunkState::Link cannot be converted to sql"),
        };
        Ok(s.into())
    }
}

impl FromSql for ChunkState {
    fn column_result(value: ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let s = value.as_str().unwrap();
        Ok(ChunkState::from_str(s))
    }
}

pub struct ChunkItem {
    pub chunk_id: ChunkId,
    pub chunk_size: u64,
    pub chunk_state: ChunkState,
    pub progress: String,
    pub description: String,
    pub create_time: u64,
    pub update_time: u64,
}

impl ChunkItem {
    pub fn new(chunk_id: &ChunkId, chunk_size: u64, description: Option<&str>) -> Self {
        let now_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        Self {
            chunk_id: chunk_id.clone(),
            chunk_size,
            chunk_state: ChunkState::New,
            progress: "".to_string(),
            description: description.unwrap_or("").to_string(),
            create_time: now_time,
            update_time: now_time,
        }
    }

    pub fn new_completed(chunk_id: &ChunkId, chunk_size: u64, description: Option<&str>) -> Self {
        let mut result = Self::new(chunk_id, chunk_size, description);
        result.chunk_state = ChunkState::Completed;
        result
    }
}

// Create a new trait that combines AsyncRead and AsyncSeek
pub trait ChunkReadSeek: AsyncRead + AsyncSeek {}

// Blanket implementation for any type that implements both traits
impl<T: AsyncRead + AsyncSeek> ChunkReadSeek for T {}
