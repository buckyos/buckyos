use std::error::Error;

use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub struct BackupChunk {
    pub seq: u32,
    pub path: String,
    pub hash: String,
    pub size: u32,
    pub relative_path: String,
}

#[derive(Deserialize, Serialize)]
pub struct BackupVersionMeta {
    pub key: String,
    pub version: u32,
    pub prev_version: Option<u32>,
    pub meta: String,
    pub is_restorable: bool,

    pub chunk_count: u32,
}

pub struct BackupIndexSqlite {
    conn: rusqlite::Connection,
}

// #[derive(PartialEq, Eq, Debug)]
// enum StatementId {
//     InsertBackup = 0,
//     InsertChunk = 1,

//     Count,
// }

// const STATEMENTS: [(StatementId, &str); StatementId::Count as usize] = [
//     (
//         StatementId::InsertBackup,
//         r#"INSERT INTO backup_version (
//             key, version, meta, chunk_count
//         ) VALUES (?,?,?,?)"#,
//     ),
//     (
//         StatementId::InsertChunk,
//         r#"INSERT INTO version_chunk (
//             key, version, chunk_seq, chunk_path, hash
//         ) VALUES (?,?,?,?,?)"#,
//     ),
// ];

impl BackupIndexSqlite {
    pub fn init(db_path: &str) -> Result<Self, Box<dyn Error>> {
        let conn = Self::create_db(db_path)?;

        Ok(Self { conn })
    }

    fn create_db(db_path: &str) -> Result<rusqlite::Connection, Box<dyn Error>> {
        let conn = rusqlite::Connection::open(db_path).map_err(|e| {
            log::warn!("open db failed, db={}, e={}", db_path, e);
            e
        })?;

        {
            let sqls = [
                r#"CREATE TABLE IF NOT EXISTS backup_version (
                    zone_id TEXT NOT NULL,
                    key TEXT NOT NULL,
                    version INTEGER NOT NULL,
                    meta TEXT DEFAULT "",
                    prev_version INTEGER DEFAULT NULL,
                    is_restorable TINYINT DEFAULT 0,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (zone_id, key, version)
                );"#,
                r#"CREATE TABLE IF NOT EXISTS version_chunk (
                    zone_id TEXT NOT NULL,
                    key TEXT NOT NULL,
                    version INTEGER NOT NULL,
                    chunk_seq INTEGER NOT NULL,
                    chunk_path TEXT NOT NULL,
                    hash TEXT NOT NULL,
                    chunk_size INTEGER NOT NULL,
                    chunk_relative_path TEXT,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (zone_id, key, version, chunk_seq)
                );"#,
            ];

            // TODO: (key, version)是外键

            for sql in sqls {
                conn.execute(sql, []).map_err(|e| e)?;
            }
        }

        // {
        //     for i in 0..STATEMENTS.len() { {
        //         let (statement_id, sql) = STATEMENTS[i];
        //         assert_eq!(statement_id, i);
        //         let statement = conn.prepare(sql)?;
        //     }
        // }

        Ok(conn)
    }

    pub fn insert_new_backup(
        &self,
        zone_id: &str,
        key: &str,
        version: u32,
        _todo_prev_version: Option<u32>,
    ) -> Result<(), Box<dyn Error>> {
        // 在表backup_version中插入新行
        let sql = r#"INSERT INTO backup_version (
            zone_id, key, version
        ) VALUES (?1,?2,?3)"#;

        match self
            .conn
            .execute(sql, rusqlite::params![zone_id, key, version])
        {
            Err(err)
                if err.sqlite_error_code() != Some(rusqlite::ErrorCode::ConstraintViolation) =>
            {
                Err(Box::new(err))
            }
            _ => Ok(()),
        }
    }

    pub fn commit_backup(
        &self,
        zone_id: &str,
        key: &str,
        version: u32,
        meta: &str,
    ) -> Result<(), Box<dyn Error>> {
        // 更新表backup_version中的is_restorable字段为true

        // 在表backup_version中插入新行
        let sql = r#"
            UPDATE backup_version
            SET is_restorable = 1, meta = ?1
            WHERE zone_id = ?2 AND key = ?3 AND version = ?4
        "#;

        match self
            .conn
            .execute(sql, rusqlite::params![meta, zone_id, key, version])
        {
            Ok(_) => Ok(()),
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn insert_new_chunk(
        &self,
        zone_id: &str,
        key: &str,
        version: u32,
        chunk_seq: u32,
        chunk_path: &str,
        hash: &str,
        chunk_size: u32,
        chunk_relative_path: &str,
    ) -> Result<(), Box<dyn Error>> {
        // 在表backup_version中插入新行
        let sql = r#"INSERT INTO version_chunk (
            zone_id, key, version, chunk_seq, chunk_path, hash, chunk_size, chunk_relative_path
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"#;

        match self.conn.execute(
            sql,
            rusqlite::params![
                zone_id,
                key,
                version,
                chunk_seq,
                chunk_path,
                hash,
                chunk_size,
                chunk_relative_path
            ],
        ) {
            Err(err)
                if err.sqlite_error_code() != Some(rusqlite::ErrorCode::ConstraintViolation) =>
            {
                Err(Box::new(err))
            }
            _ => Ok(()),
        }
    }

    pub fn query_backup_versions(
        &self,
        zone_id: &str,
        key: &str,
        offset: i32,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<BackupVersionMeta>, Box<dyn Error>> {
        let is_asc = offset >= 0;
        let (sql, offset, limit) = if offset > 0 {
            (
                r#"SELECT backup_version.version, backup_version.meta, backup_version.is_restorable, backup_version.prev_version, COUNT(*) AS chunk_count
                FROM backup_version, version_chunk
                WHERE backup_version.zone_id = version_chunk.zone_id AND 
                    backup_version.key = version_chunk.key AND 
                    backup_version.version = version_chunk.version AND 
                    backup_version.zone_id = ?1 AND backup_version.key = ?2 AND (?3 OR backup_version.is_restorable = 1)
                GROUP BY backup_version.zone_id, backup_version.key, backup_version.version
                ORDER BY backup_version.version ASC
                LIMIT ?4 OFFSET ?5
            "#,
                offset,
                limit,
            )
        } else {
            (
                r#"SELECT backup_version.version, backup_version.meta, backup_version.is_restorable, backup_version.prev_version, COUNT(*) AS chunk_count
                FROM backup_version, version_chunk
                WHERE backup_version.zone_id = version_chunk.zone_id AND 
                    backup_version.key = version_chunk.key AND 
                    backup_version.version = version_chunk.version AND 
                    backup_version.zone_id = ?1 AND backup_version.key = ?2 AND (?3 OR backup_version.is_restorable = 1)
                GROUP BY backup_version.zone_id, backup_version.key, backup_version.version
                ORDER BY backup_version.version DESC
                LIMIT ?4 OFFSET ?5
            "#,
                std::cmp::max(-offset - (limit as i32), 0),
                std::cmp::min(-offset as u32, limit),
            )
        };

        let mut stmt = self.conn.prepare(sql)?;
        let version_rows = stmt
            .query_map(
                rusqlite::params![zone_id, key, !is_restorable_only, limit, offset],
                |row| {
                    Ok(BackupVersionMeta {
                        key: key.to_string(),
                        version: row.get(0).unwrap(),
                        meta: row.get(1).unwrap(),
                        is_restorable: row.get::<usize, u8>(2).unwrap() == 1,
                        prev_version: row.get(3).unwrap(),
                        chunk_count: row.get(4).unwrap(),
                    })
                },
            )?
            .collect::<Vec<_>>();

        let mut versions = vec![];

        for row in version_rows {
            if is_asc {
                versions.push(row.unwrap());
            } else {
                versions.insert(0, row.unwrap())
            }
        }

        Ok(versions)
    }

    pub fn query_backup_version_info(
        &self,
        zone_id: &str,
        key: &str,
        version: u32,
    ) -> Result<BackupVersionMeta, Box<dyn Error>> {
        log::info!(
            "query_backup_version_info: zone_id={}, key={}, version={}",
            zone_id,
            key,
            version
        );
        let sql = r#"SELECT backup_version.meta, backup_version.is_restorable, backup_version.prev_version, COUNT(*) AS chunk_count
                FROM backup_version, version_chunk
                WHERE backup_version.zone_id = version_chunk.zone_id AND 
                    backup_version.key = version_chunk.key AND 
                    backup_version.version = version_chunk.version AND 
                    backup_version.zone_id = ?1 AND backup_version.key = ?2 AND backup_version.version = ?3
                GROUP BY backup_version.zone_id, backup_version.key, backup_version.version
            "#;

        let version_info =
            self.conn
                .query_row(sql, rusqlite::params![zone_id, key, version], |row| {
                    Ok(BackupVersionMeta {
                        key: key.to_string(),
                        version,
                        meta: row.get(0).unwrap(),
                        is_restorable: row.get::<usize, u8>(1).unwrap() == 1,
                        prev_version: row.get(2).unwrap(),
                        chunk_count: row.get(3).unwrap(),
                    })
                })?;

        Ok(version_info)
    }

    pub fn query_chunk(
        &self,
        zone_id: &str,
        key: &str,
        version: u32,
        chunk_seq: u32,
    ) -> Result<BackupChunk, Box<dyn Error>> {
        let sql = r#"
            SELECT chunk_path, hash, chunk_size, chunk_relative_path
            FROM version_chunk
            WHERE zone_id = ?1 AND key = ?2 AND version = ?3 AND chunk_seq = ?4
        "#;

        let chunk = self.conn.query_row(
            sql,
            rusqlite::params![zone_id, key, version, chunk_seq],
            |row| {
                Ok(BackupChunk {
                    seq: chunk_seq,
                    path: row.get(0).unwrap(),
                    hash: row.get(1).unwrap(),
                    size: row.get(2).unwrap(),
                    relative_path: row.get(3).unwrap(),
                })
            },
        )?;

        Ok(chunk)
    }
}
