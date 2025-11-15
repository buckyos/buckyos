use rusqlite::{Connection, OptionalExtension, Result as SqlResult};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct LogFileEntry {
    pub id: i64,
    pub name: String,
    pub create_time: i64,
    pub write_index: i64,
    pub is_sealed: bool,

    pub read_index: i64,
    pub is_read_complete: bool,
}

#[derive(Debug, Clone)]
pub struct LogFileWriteInfo {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct LogFileReadInfo {
    pub id: i64,
    pub name: String,
    pub is_sealed: bool,
    pub read_index: i64,
    pub is_read_complete: bool,
}

pub struct LogMeta {
    log_dir: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl LogMeta {
    pub fn open(log_dir: &Path) -> Result<Self, String> {
        let db_path = log_dir.join("log_meta.db");
        let conn = Connection::open(db_path).map_err(|e| {
            let msg = format!("failed to open log meta db: {}", e);
            error!("{}", msg);
            msg
        })?;

        // Init the database schema if not exists
        conn.execute(
            "CREATE TABLE IF NOT EXISTS LogFiles (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                create_time INTEGER NOT NULL,
                write_index INTEGER NOT NULL,
                is_sealed INTEGER NOT NULL,
                read_index INTEGER NOT NULL,
                is_read_complete INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| {
            let msg = format!("failed to create log meta table: {}", e);
            error!("{}", msg);
            msg
        })?;

        println!("LogMeta initialized successfully {}", log_dir.display());

        Ok(LogMeta {
            conn: Arc::new(Mutex::new(conn)),
            log_dir: log_dir.to_path_buf(),
        })
    }

    pub fn get_file_info(&self, id: i64) -> SqlResult<Option<LogFileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, create_time, write_index, is_sealed, read_index, is_read_complete 
             FROM LogFiles 
             WHERE id = ?1",
        )?;

        let file_option = stmt
            .query_row([id], |row| {
                Ok(LogFileEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    create_time: row.get(2)?,
                    write_index: row.get(3)?,
                    is_sealed: row.get(4)?,
                    read_index: row.get(5)?,
                    is_read_complete: row.get(6)?,
                })
            })
            .optional()?; // Use optional to handle no rows case

        Ok(file_option)
    }

    pub fn append_new_file(&self, file_name: &str) -> SqlResult<()> {
        let current_file = self.get_active_write_file()?;
        if current_file.is_some() {
            let msg = format!(
                "there is already an active write file, cannot append new one: {}",
                file_name
            );
            error!("{}", msg);
            return Err(rusqlite::Error::InvalidQuery); // Or some other appropriate error
        }

        let conn = self.conn.lock().unwrap();
        conn.execute(
        "INSERT INTO LogFiles (name, create_time, write_index, is_sealed, read_index, is_read_complete) 
            VALUES (?1, strftime('%s','now'), 0, 0, 0, 0)",
            &[file_name],
        )?;

        Ok(())
    }

    // Get the current active write file (not sealed) if exists
    pub fn get_active_write_file(&self) -> SqlResult<Option<LogFileWriteInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt: rusqlite::Statement<'_> = conn.prepare(
            "SELECT id, name FROM LogFiles WHERE is_sealed = 0 ORDER BY id DESC LIMIT 1",
        )?;

        let file_option = stmt
            .query_row([], |row| {
                Ok(LogFileWriteInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .optional()?; // Use optional to handle no rows case

        Ok(file_option)
    }

    pub fn get_last_sealed_file(&self) -> SqlResult<Option<LogFileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, create_time, write_index, is_sealed, read_index, is_read_complete 
             FROM LogFiles 
             WHERE is_sealed = 1 
             ORDER BY id DESC LIMIT 1",
        )?;

        let file_option = stmt
            .query_row([], |row| {
                Ok(LogFileEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    create_time: row.get(2)?,
                    write_index: row.get(3)?,
                    is_sealed: row.get(3)?,
                    read_index: row.get(4)?,
                    is_read_complete: row.get(5)?,
                })
            })
            .optional()?; // Use optional to handle no rows case

        Ok(file_option)
    }

    pub fn update_current_write_index(&self, new_index: u64) -> SqlResult<()> {
        let current_file = self.get_active_write_file()?;
        if let Some(file) = current_file {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE LogFiles SET write_index = ?1 WHERE id = ?2",
                &[&(new_index as i64), &file.id],
            )?;

            /*
            println!(
                "Updated write index for log file: {}, {} to {}",
                file.id, file.name, new_index
            );
            */

            Ok(())
        } else {
            let msg = "no active write file to update index";
            error!("{}", msg);
            Err(rusqlite::Error::InvalidQuery) // Or some other appropriate error
        }
    }

    // Increase the current write index by a given amount
    pub fn increase_current_write_index(&self, increment: i64) -> SqlResult<()> {
        let current_file = self.get_active_write_file()?;
        if let Some(file) = current_file {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE LogFiles SET write_index = write_index + ?1 WHERE id = ?2",
                &[&increment, &file.id],
            )?;

            println!(
                "Increased write index for log file: {}, {} by {}",
                file.id, file.name, increment
            );

            Ok(())
        } else {
            let msg = "no active write file to increase index";
            error!("{}", msg);
            Err(rusqlite::Error::InvalidQuery) // Or some other appropriate error
        }
    }

    pub fn seal_current_write_file(&self) -> SqlResult<()> {
        let current_file = self.get_active_write_file()?;
        if let Some(file) = current_file {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE LogFiles SET is_sealed = 1 WHERE id = ?1",
                &[&file.id],
            )?;

            info!("Sealed log file: {}, {}", file.id, file.name);
            Ok(())
        } else {
            let msg = "no active write file to seal";
            error!("{}", msg);
            Err(rusqlite::Error::InvalidQuery) // Or some other appropriate error
        }
    }

    pub fn get_active_read_file(&self) -> SqlResult<Option<LogFileReadInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, is_sealed, read_index, is_read_complete 
             FROM LogFiles 
             WHERE is_read_complete = 0 
             ORDER BY id ASC LIMIT 1",
        )?;

        let file_option = stmt
            .query_row([], |row| {
                Ok(LogFileReadInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_sealed: row.get(2)?,
                    read_index: row.get(3)?,
                    is_read_complete: row.get(4)?,
                })
            })
            .optional()?; // Use optional to handle no rows case

        Ok(file_option)
    }

    pub fn update_current_read_index(&self, new_index: i64) -> SqlResult<()> {
        let current_file = self.get_active_read_file()?;
        if let Some(file) = current_file {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE LogFiles SET read_index = ?1 WHERE id = ?2",
                &[&new_index, &file.id],
            )?;

            info!(
                "Updated read index for log file: {}, {} to {}",
                file.id, file.name, new_index
            );

            Ok(())
        } else {
            let msg = "no active read file to update index";
            error!("{}", msg);
            Err(rusqlite::Error::InvalidQuery) // Or some other appropriate error
        }
    }

    pub fn update_file_read_index(&self, file_id: i64, new_index: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let ret = conn.execute(
            "UPDATE LogFiles SET read_index = ?1 WHERE id = ?2",
            &[&new_index, &file_id],
        )?;

        if ret == 0 {
            let msg = format!("no log file found with id: {}", file_id);
            error!("{}", msg);
            return Err(rusqlite::Error::InvalidQuery); // Or some other appropriate error
        }

        info!(
            "Updated read index for log file: {} to {}",
            file_id, new_index
        );

        Ok(())
    }

    pub fn complete_current_read_file(&self) -> SqlResult<()> {
        let current_file = self.get_active_read_file()?;
        if let Some(file) = current_file {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE LogFiles SET is_read_complete = 1 WHERE id = ?1",
                &[&file.id],
            )?;

            info!("Completed reading log file: {}, {}", file.id, file.name);

            Ok(())
        } else {
            let msg = "no active read file to complete";
            error!("{}", msg);
            Err(rusqlite::Error::InvalidQuery) // Or some other appropriate error
        }
    }

    pub fn mark_file_read_complete(&self, file_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE LogFiles SET is_read_complete = 1 WHERE id = ?1",
            &[&file_id],
        )?;

        info!("Marked log file read complete: {}", file_id);

        Ok(())
    }
}
