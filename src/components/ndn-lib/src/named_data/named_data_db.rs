use std::pin::Pin;
use std::{collections::HashMap, io::SeekFrom};
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::json;
use tokio::{
    fs::{self, File,OpenOptions}, 
    io::{self, AsyncRead,AsyncWrite, AsyncReadExt, AsyncWriteExt, AsyncSeek, AsyncSeekExt}, 
};
use log::*;
use rusqlite::{params, Connection, Result as SqliteResult};
use rusqlite::types::{ToSql, FromSql, ValueRef};
use async_trait::async_trait;
use tokio::sync::Mutex;
use name_lib::EncodedDocument;
use crate::{ChunkReader,ChunkWriter,ChunkHasher, ChunkId, LinkData, NdnError, NdnResult, ObjId, ObjectLink};
use super::def::{ChunkItem, ChunkState};

pub(crate) struct NamedDataDb {
    db_path: String,
    conn: Mutex<Connection>,
}

impl NamedDataDb {
    pub fn new(db_path: String) -> NdnResult<Self> {
        // Add OpenOptions to ensure we have write permissions
        info!("NamedDataDb: db_path: {}", db_path);
        let conn = Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| {
            warn!("NamedDataDb: open db failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunk_items (
                chunk_id TEXT PRIMARY KEY,
                chunk_size INTEGER NOT NULL, 
                chunk_state TEXT NOT NULL,
                progress TEXT,
                description TEXT NOT NULL,
                create_time INTEGER NOT NULL,
                update_time INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| {
            warn!(
                "NamedDataDb: create table chunk_items failed! {}",
                e.to_string()
            );
            NdnError::DbError(e.to_string())
        })?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS objects (
                obj_id TEXT PRIMARY KEY,
                obj_type TEXT NOT NULL,
                obj_data TEXT NOT NULL,
                create_time INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| {
            warn!(
                "NamedDataDb: create objects table failed! {}",
                e.to_string()
            );
            NdnError::DbError(e.to_string())
        })?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS object_links (
                link_obj_id TEXT PRIMARY KEY,
                obj_link TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| {
            warn!(
                "NamedDataDb: create table object_links failed! {}",
                e.to_string()
            );
            NdnError::DbError(e.to_string())
        })?;

        Ok(Self {
            db_path,
            conn: Mutex::new(conn),
        })
    }

    pub async fn set_chunk_item(&self, chunk_item: &ChunkItem) -> NdnResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO chunk_items 
            (chunk_id, chunk_size, chunk_state, progress, 
             description, create_time, update_time)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                chunk_item.chunk_id.to_string(),
                chunk_item.chunk_size,
                chunk_item.chunk_state,
                chunk_item.progress,
                chunk_item.description,
                chunk_item.create_time,
                chunk_item.update_time,
            ],
        )
        .map_err(|e| {
            warn!("NamedDataDb: insert chunk failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub async fn get_chunk(&self, chunk_id: &ChunkId) -> NdnResult<ChunkItem> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare("SELECT * FROM chunk_items WHERE chunk_id = ?1")
            .map_err(|e| {
                //warn!("NamedDataDb: get_chunk failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        let chunk = stmt
            .query_row(params![chunk_id.to_string()], |row| {
                Ok(ChunkItem {
                    chunk_id: chunk_id.clone(),
                    chunk_size: row.get(1)?,
                    chunk_state: row.get(2)?,
                    progress: row.get(3)?,
                    description: row.get(4)?,
                    create_time: row.get(5)?,
                    update_time: row.get(6)?,
                })
            })
            .map_err(|e| {
                warn!("ChunkDb: get_chunk failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        Ok(chunk)
    }

    pub async fn put_chunk_list(&self, chunk_list: Vec<ChunkItem>) -> NdnResult<()> {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkDb: transaction failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        for chunk in chunk_list {
            tx.execute(
                "INSERT OR REPLACE INTO chunk_items 
                (chunk_id, chunk_size, chunk_state, progress, description, create_time, update_time)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    chunk.chunk_id.to_string(),
                    chunk.chunk_size,
                    chunk.chunk_state,
                    chunk.progress,
                    chunk.description,
                    chunk.create_time,
                    chunk.update_time,
                ],
            )
            .map_err(|e| {
                warn!("NamedDataDb: insert chunk failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;
        }

        tx.commit().map_err(|e| {
            warn!("ChunkDb: commit failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        Ok(())
    }

    pub async fn update_chunk_progress(&self, chunk_id: &ChunkId, progress: String) -> NdnResult<()> {
        let mut conn = self.conn.lock().await;
        conn.execute(
            "UPDATE chunk_items SET progress = ?1, chunk_state = 'incompleted', update_time = CURRENT_TIMESTAMP WHERE chunk_id = ?2",
            params![progress, chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkDb: update chunk progress failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub async fn remove_chunk(&self, chunk_id: &ChunkId) -> NdnResult<()> {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkDb: transaction failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // First remove any links pointing to this chunk
        // tx.execute(
        //     "DELETE FROM chunk_links WHERE target_chunk_id = ?1",
        //     params![chunk_id.to_string()],
        // ).map_err(|e| {
        //     warn!("ChunkDb: delete link failed! {}", e.to_string());
        //     NdnError::DbError(e.to_string())
        // })?;

        // Then remove the chunk itself
        tx.execute(
            "DELETE FROM chunk_items WHERE chunk_id = ?1",
            params![chunk_id.to_string()],
        )
        .map_err(|e| {
            warn!("ChunkDb: delete chunk failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        tx.commit().map_err(|e| {
            warn!("ChunkDb: commit failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub async fn set_object(&self, obj_id: &ObjId, obj_type: &str, obj_str: &str) -> NdnResult<()> {
        let now_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO objects (obj_id, obj_type, obj_data, create_time)
             VALUES (?1, ?2, ?3, ?4)",
            params![obj_id.to_string(), obj_type, obj_str, now_time],
        )
        .map_err(|e| {
            warn!("ChunkDb: insert object failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub async fn get_object(&self, obj_id: &ObjId) -> NdnResult<(String, String)> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare("SELECT obj_type,obj_data FROM objects WHERE obj_id = ?1")
            .map_err(|e| {
                warn!("ChunkDb: query object failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        let obj_data = stmt
            .query_row(params![obj_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| {
                warn!("ChunkDb: query object failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        Ok(obj_data)
    }

    pub async fn remove_object(&self, obj_id: &ObjId) -> NdnResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM objects WHERE obj_id = ?1",
            params![obj_id.to_string()],
        )
        .map_err(|e| {
            warn!("ChunkDb: remove object failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        Ok(())
    }

    pub async fn set_object_link(&self, obj_id: &ObjId, obj_link: &LinkData) -> NdnResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO object_links (link_obj_id, obj_link)
             VALUES (?1, ?2)",
            params![obj_id.to_string(), obj_link.to_string()],
        )
        .map_err(|e| {
            warn!("ChunkDb: insert object link failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub async fn query_object_link_ref(&self, ref_obj_id: &ObjId) -> NdnResult<Vec<String>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare("SELECT link_obj_id FROM object_links WHERE obj_link LIKE ?1")
            .map_err(|e| {
                warn!("NamedDataDb: query object link failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        let ref_obj_id_str = format!("%{}%", ref_obj_id.to_string());
        let mut rows = stmt.query(params![ref_obj_id_str]).map_err(|e| {
            warn!("NamedDataDb: query object link failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        let mut link_obj_ids = Vec::new();
        while let Some(row) = rows.next().map_err(|e| {
            warn!("NamedDataDb: query object link failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })? {
            let link_obj_id: String = row.get(0).map_err(|e| {
                warn!("NamedDataDb: query object link failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;
            link_obj_ids.push(link_obj_id);
        }

        Ok(link_obj_ids)
    }

    pub async fn get_object_link(&self, obj_id: &ObjId) -> NdnResult<String> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare("SELECT obj_link FROM object_links WHERE link_obj_id = ?1")
            .map_err(|e| {
                warn!("NamedDataDb: query object link failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        let obj_link = stmt
            .query_row(params![obj_id.to_string()], |row| row.get::<_, String>(0))
            .map_err(|e| {
                warn!("NamedDataDb: query object link failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;
        Ok(obj_link)
    }

    pub async fn remove_object_link(&self, obj_id: &ObjId) -> NdnResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM object_links WHERE link_obj_id = ?1",
            params![obj_id.to_string()],
        )
        .map_err(|e| {
            warn!("ChunkDb: remove object link failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }
}
