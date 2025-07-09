use crate::{
    build_named_object_by_json, ChunkHasher, ChunkId, ChunkListReader, ChunkReadSeek, ChunkState,
    FileObject, NamedDataStore, NdnError, NdnResult, PathObject,
};
use buckyos_kit::get_buckyos_named_data_dir;
use buckyos_kit::{
    buckyos_get_unix_timestamp, get_buckyos_root_dir, get_by_json_path, get_relative_path,
};
use futures_util::stream;
use futures_util::stream::StreamExt;
use lazy_static::lazy_static;
use log::*;
use memmap::Mmap;
use name_lib::decode_jwt_claim_without_verify;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::io as std_io;
use std::io::SeekFrom;
use std::sync::Arc;
use std::sync::Mutex;
use std::{path::PathBuf, pin::Pin};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{self, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt},
};
use tokio_util::bytes::BytesMut;
use tokio_util::io::StreamReader;

use crate::{ChunkList, ChunkReader, ChunkWriter, ObjId};

impl From<NdnError> for std_io::Error {
    fn from(err: NdnError) -> Self {
        std_io::Error::new(std_io::ErrorKind::Other, err.to_string())
    }
}
pub struct NamedDataMgrDB {
    db_path: String,
    conn: Mutex<Connection>,
}

impl NamedDataMgrDB {
    pub fn new(db_path: String) -> NdnResult<Self> {
        let conn = Connection::open(&db_path).map_err(|e| {
            warn!("NamedDataMgrDB: open db failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS objs (
                obj_id TEXT PRIMARY KEY,
                ref_count INTEGER NOT NULL DEFAULT 0,
                access_time INTEGER NOT NULL,
                size INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .map_err(|e| {
            warn!(
                "NamedDataMgrDB: create objs table failed! {}",
                e.to_string()
            );
            NdnError::DbError(e.to_string())
        })?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS paths (
                path TEXT PRIMARY KEY,
                obj_id TEXT NOT NULL,
                path_obj_jwt TEXT,
                app_id TEXT NOT NULL,
                user_id TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| {
            warn!(
                "NamedDataMgrDB: create paths table failed! {}",
                e.to_string()
            );
            NdnError::DbError(e.to_string())
        })?;

        // 添加索引
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_paths_path ON paths(path)",
            [],
        )
        .map_err(|e| {
            warn!("NamedDataMgrDB: create index failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        Ok(Self {
            db_path,
            conn: Mutex::new(conn),
        })
    }

    // 路径规范化函数
    pub fn normalize_path(path: &str) -> String {
        path.replace("//", "/").trim_start_matches("./").to_string()
    }

    //return (result_path, obj_id,path_obj_jwt,relative_path)
    pub fn find_longest_matching_path(
        &self,
        path: &str,
    ) -> NdnResult<(String, ObjId, Option<String>, Option<String>)> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare("SELECT path, obj_id,path_obj_jwt FROM paths WHERE ? LIKE (path || '%') ORDER BY length(path) DESC LIMIT 1")
            .map_err(|e| {
                warn!("NamedDataMgrDB: prepare statement failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        let record: (String, String, Option<String>) = stmt
            .query_row([path], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| {
                warn!(
                    "NamedDataMgrDB: query {} obj id failed! {}",
                    path,
                    e.to_string()
                );
                NdnError::DbError(e.to_string())
            })?;

        let result_path = record.0;
        let obj_id_str = record.1;
        let path_obj_jwt = record.2;
        if path_obj_jwt.is_some() {
            info!(
                "NamedDataMgrDB: find_longest_matching_path, path_obj_jwt {}",
                path_obj_jwt.as_ref().unwrap()
            );
        }
        let obj_id = ObjId::new(&obj_id_str)?;
        let relative_path = get_relative_path(&result_path, path);
        Ok((result_path, obj_id, path_obj_jwt, Some(relative_path)))
    }

    pub fn get_path_target_objid(&self, path: &str) -> NdnResult<(ObjId, Option<String>)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT obj_id,path_obj_jwt FROM paths WHERE path = ?1")
            .map_err(|e| {
                warn!(
                    "NamedDataMgrDB: prepare statement failed! {}",
                    e.to_string()
                );
                NdnError::DbError(e.to_string())
            })?;

        let (obj_id_str, path_obj_jwt): (String, Option<String>) = stmt
            .query_row([path], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| {
                warn!(
                    "NamedDataMgrDB: query {} target obj failed! {}",
                    path,
                    e.to_string()
                );
                NdnError::DbError(e.to_string())
            })?;

        let obj_id = ObjId::new(&obj_id_str).map_err(|e| {
            warn!("NamedDataMgrDB: invalid obj_id format! {}", e.to_string());
            NdnError::Internal(e.to_string())
        })?;

        Ok((obj_id, path_obj_jwt))
    }

    pub fn update_obj_access_time(&self, obj_id: &ObjId, access_time: u64) -> NdnResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE objs SET access_time = ?1 WHERE obj_id = ?2",
            [&access_time.to_string(), &obj_id.to_string()],
        )
        .map_err(|e| {
            warn!(
                "NamedDataMgrDB: update obj access time failed! {}",
                e.to_string()
            );
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn create_path(
        &self,
        obj_id: &ObjId,
        path: &str,
        app_id: &str,
        user_id: &str,
    ) -> NdnResult<()> {
        if path.len() < 2 {
            return Err(NdnError::InvalidParam(
                "path length must be greater than 2".to_string(),
            ));
        }

        let mut conn = self.conn.lock().unwrap();
        let obj_id = obj_id.to_string();
        let tx = conn.transaction().map_err(|e| {
            warn!(
                "NamedDataMgrDB: tx.transaction error, create path failed! {}",
                e.to_string()
            );
            NdnError::DbError(e.to_string())
        })?;

        tx.execute(
            "INSERT INTO paths (path, obj_id, app_id, user_id) VALUES (?1, ?2, ?3, ?4)",
            [&path, obj_id.as_str(), app_id, user_id],
        )
        .map_err(|e| {
            warn!(
                "NamedDataMgrDB: tx.execute error, create path failed! {:?}",
                &e
            );

            NdnError::DbError(e.to_string())
        })?;

        tx.commit().map_err(|e| {
            warn!(
                "NamedDataMgrDB:tx.commit error, create path failed! {}",
                e.to_string()
            );
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn set_path(
        &self,
        path: &str,
        new_obj_id: &ObjId,
        path_obj_str: String,
        app_id: &str,
        user_id: &str,
    ) -> NdnResult<()> {
        //如果不存在路径则创建，否则更新已经存在的路径指向的chunk
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Check if the path exists
        let existing_obj_id: Result<String, _> =
            tx.query_row("SELECT obj_id FROM paths WHERE path = ?1", [&path], |row| {
                row.get(0)
            });

        let obj_id_str = new_obj_id.to_string();

        match existing_obj_id {
            Ok(obj_id) => {
                // Path exists, update the obj_id
                tx.execute(
                    "UPDATE paths SET obj_id = ?1, path_obj_jwt = ?2, app_id = ?3, user_id = ?4 WHERE path = ?5",
                    [obj_id_str.as_str(), path_obj_str.as_str(), app_id, user_id, &path],
                ).map_err(|e| {
                    warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
                    NdnError::DbError(e.to_string())
                })?;
            }
            Err(_) => {
                // Path does not exist, create a new path
                tx.execute(
                    "INSERT INTO paths (path, obj_id, path_obj_jwt, app_id, user_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                    [&path, obj_id_str.as_str(), path_obj_str.as_str(), app_id, user_id],
                ).map_err(|e| {
                    warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
                    NdnError::DbError(e.to_string())
                })?;
            }
        }

        tx.commit().map_err(|e| {
            warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn remove_path(&self, path: &str) -> NdnResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Get the chunk_id for this path
        let obj_id: String = tx
            .query_row("SELECT obj_id FROM paths WHERE path = ?1", [&path], |row| {
                row.get(0)
            })
            .map_err(|e| {
                warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        // Remove the path
        tx.execute("DELETE FROM paths WHERE path = ?1", [&path])
            .map_err(|e| {
                warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        tx.commit().map_err(|e| {
            warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn remove_dir_path(&self, path: &str) -> NdnResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Get all paths and their chunk_ids that start with the given directory path
        let mut stmt = tx
            .prepare("SELECT path, obj_id FROM paths WHERE path LIKE ?1")
            .map_err(|e| {
                warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        let rows = stmt
            .query_map([format!("{}%", path)], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| {
                warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

        let path_objs: Vec<(String, String)> = rows.filter_map(Result::ok).collect();

        // Remove paths and update chunk ref counts within the transaction
        for (path, obj_id) in path_objs {
            // Remove the path
            tx.execute("DELETE FROM paths WHERE path = ?1", [&path])
                .map_err(|e| {
                    warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
                    NdnError::DbError(e.to_string())
                })?;
        }

        drop(stmt);
        tx.commit().map_err(|e| {
            warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        Ok(())
    }

    pub fn set_path_obj_jwt(&self, path: &str, path_obj_jwt: &str) -> NdnResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE paths SET path_obj_jwt = ?1 WHERE path = ?2",
            [path_obj_jwt, path],
        )
        .map_err(|e| {
            warn!("NamedDataMgrDB: set path obj jwt failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }
}
