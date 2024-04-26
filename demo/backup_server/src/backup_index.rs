use std::error::Error;

use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub struct BackupChunk {
    pub seq: u32,
    pub path: String,
    pub hash: String,
    pub size: u32,
}

#[derive(Deserialize, Serialize)]
pub struct BackupVersionMeta {
    pub key: String,
    pub version: u32,
    pub meta: String,
    pub chunk_count: u32,

    pub chunks: Vec<BackupChunk>,
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
                    key TEXT NOT NULL,
                    version INTEGER NOT NULL,
                    meta TEXT,
                    chunk_count INTEGER,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (key, version)
                );"#,
                r#"CREATE TABLE IF NOT EXISTS version_chunk (
                    key TEXT NOT NULL,
                    version INTEGER NOT NULL,
                    chunk_seq INTEGER NOT NULL,
                    chunk_path TEXT NOT NULL,
                    hash TEXT NOT NULL,
                    chunk_size INTEGER NOT NULL,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (key, version, chunk_seq)
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
        key: &str,
        version: u32,
        meta: &str,
        chunk_count: u32,
    ) -> Result<(), Box<dyn Error>> {
        // 在表backup_version中插入新行
        let sql = r#"INSERT INTO backup_version (
            key, version, meta, chunk_count
        ) VALUES (?1,?2,?3,?4)"#;

        match self
            .conn
            .execute(sql, rusqlite::params![key, version, meta, chunk_count])
        {
            Err(err)
                if err.sqlite_error_code() != Some(rusqlite::ErrorCode::ConstraintViolation) =>
            {
                Err(Box::new(err))
            }
            _ => Ok(()),
        }
    }

    pub fn insert_new_chunk(
        &self,
        key: &str,
        version: u32,
        chunk_seq: u32,
        chunk_path: &str,
        hash: &str,
        chunk_size: u32,
    ) -> Result<(), Box<dyn Error>> {
        // 在表backup_version中插入新行
        let sql = r#"INSERT INTO version_chunk (
            key, version, chunk_seq, chunk_path, hash, chunk_size
        ) VALUES (?1,?2,?3,?4,?5,?6)"#;

        match self.conn.execute(
            sql,
            rusqlite::params![key, version, chunk_seq, chunk_path, hash, chunk_size],
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
        key: &str,
        offset: i32,
        limit: u32,
    ) -> Result<Vec<BackupVersionMeta>, Box<dyn Error>> {
        let (sql, offset, limit) = if offset > 0 {
            (
                r#"SELECT version, meta, chunk_count
                FROM backup_version
                WHERE key = ?1
                ORDER BY version ASC
                LIMIT ?2 OFFSET ?3
            "#,
                offset,
                limit,
            )
        } else {
            (
                r#"SELECT version, meta, chunk_count
                FROM backup_version
                WHERE key = ?1
                ORDER BY version DESC
                LIMIT ?2 OFFSET ?3
            "#,
                std::cmp::max(-offset - (limit as i32), 0),
                std::cmp::min(-offset as u32, limit),
            )
        };

        let mut stmt = self.conn.prepare(sql)?;
        let version_rows = stmt
            .query_map(rusqlite::params![key, limit, offset], |row| {
                Ok(BackupVersionMeta {
                    key: key.to_string(),
                    version: row.get(0).unwrap(),
                    meta: row.get(1).unwrap(),
                    chunk_count: row.get(2).unwrap(),
                    chunks: vec![],
                })
            })?
            .collect::<Vec<_>>();

        let mut versions = vec![];

        for row in version_rows {
            if offset >= 0 {
                versions.push(row.unwrap());
            } else {
                versions.insert(0, row.unwrap())
            }
        }

        if versions.len() == 0 {
            return Ok(vec![]);
        }

        let min_version = versions.first().unwrap().version;
        let max_version = versions.last().unwrap().version;

        let sql = r#"
            SELECT version, chunk_seq, chunk_path, hash, chunk_size
            FROM version_chunk
            WHERE key = ?1 AND version >= ?2 AND version <= ?3
            ORDER BY version ASC, chunk_seq ASC
        "#;

        let mut stmt = self.conn.prepare(sql)?;
        let chunks = stmt
            .query_map(rusqlite::params![key, min_version, max_version], |row| {
                Ok((
                    row.get::<usize, u32>(0).unwrap(),
                    BackupChunk {
                        seq: row.get(1).unwrap(),
                        path: row.get(2).unwrap(),
                        hash: row.get(3).unwrap(),
                        size: row.get(4).unwrap(),
                    },
                ))
            })?
            .collect::<Vec<_>>();

        if chunks.len() == 0 {
            return Ok(versions);
        }

        let mut chunk_index = 0;
        for version in versions.iter_mut() {
            for i in chunk_index..chunks.len() {
                let (v, chunk) = chunks.get(i).unwrap().as_ref().unwrap();
                if *v != version.version {
                    chunk_index = i;
                    break;
                }
                version.chunks.push(chunk.clone());
            }
        }

        Ok(versions)
    }

    pub fn query_backup_version_info(
        &self,
        key: &str,
        version: u32,
    ) -> Result<BackupVersionMeta, Box<dyn Error>> {
        let sql = r#"SELECT meta, chunk_count
                FROM backup_version
                WHERE key = ?1 AND version = ?2
            "#;

        let mut version_info =
            self.conn
                .query_row(sql, rusqlite::params![key, version], |row| {
                    Ok(BackupVersionMeta {
                        key: key.to_string(),
                        version,
                        meta: row.get(0).unwrap(),
                        chunk_count: row.get(1).unwrap(),
                        chunks: vec![],
                    })
                })?;

        let sql = r#"
            SELECT chunk_seq, chunk_path, hash, chunk_size
            FROM version_chunk
            WHERE key = ? 1 AND version = ?2
        "#;

        let mut stmt = self.conn.prepare(sql)?;
        let chunks = stmt
            .query_map(rusqlite::params![key, version], |row| {
                Ok(BackupChunk {
                    seq: row.get(0).unwrap(),
                    path: row.get(1).unwrap(),
                    hash: row.get(2).unwrap(),
                    size: row.get(3).unwrap(),
                })
            })?
            .collect::<Vec<_>>();

        if chunks.len() == 0 {
            return Ok(version_info);
        }

        version_info.chunks = chunks.into_iter().map(|ck| ck.unwrap()).collect();

        Ok(version_info)
    }

    pub fn query_chunk(
        &self,
        key: &str,
        version: u32,
        chunk_seq: u32,
    ) -> Result<BackupChunk, Box<dyn Error>> {
        let sql = r#"
            SELECT chunk_path, hash, chunk_size
            FROM version_chunk
            WHERE key = ? 1 AND version = ?2 AND chunk_seq = ?3
        "#;

        let chunk =
            self.conn
                .query_row(sql, rusqlite::params![key, version, chunk_seq], |row| {
                    Ok(BackupChunk {
                        seq: chunk_seq,
                        path: row.get(0).unwrap(),
                        hash: row.get(1).unwrap(),
                        size: row.get(2).unwrap(),
                    })
                })?;

        Ok(chunk)
    }
}
