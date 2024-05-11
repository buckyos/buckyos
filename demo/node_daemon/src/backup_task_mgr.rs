use std::{collections::HashMap, path::Path};

pub struct BackupTask {
    files: Vec<String>,
}

impl BackupTask {
    fn new(files: Vec<String>) -> BackupTask {
        unimplemented!()
    }

    // append new chunks to the task in order.
    pub async fn append_chunk_files(
        &self,
        chunk_file_paths: &[&Path],
    ) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!()
    }

    // all chunks has appended to the task, and there will be no chunks any more to append.
    pub async fn all_chunks_ready(
        &self,
        meta: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!()
    }

    // should not append any chunks to the task when all chunks ready.
    pub async fn is_all_chunks_ready(&self) -> bool {
        unimplemented!()
    }

    // all chunks appended to the task has been uploaded to the backup server.
    // but if a new chunk is appended to the task, the task will be not done.
    // is_all_chunks_ready && is_all_chunks_backup_done == true => the task is done.
    pub async fn is_all_chunks_backup_done(&self) -> bool {
        unimplemented!()
    }

    // remove the task and all chunks files(if is_delete_chunk_files is true).
    // and remove the task on the backup server if is_remove_local_only is false.
    // if the task is referenced by any follow tasks, the function will fail.
    pub async fn cancel(
        &self,
        is_remove_local_only: bool,
        is_delete_chunk_files: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!()
    }

    pub async fn key(&self) -> &str {
        unimplemented!()
    }

    pub async fn version(&self) -> u32 {
        unimplemented!()
    }

    pub async fn prev_version(&self) -> Option<u32> {
        unimplemented!()
    }
    pub async fn metadata(&self) -> Option<String> {
        unimplemented!()
    }
}

pub struct TaskManager {
    tasks: HashMap<String, BackupTask>,
}

impl TaskManager {
    pub fn new() -> Self {
        TaskManager {
            tasks: HashMap::new(),
        }
    }

    pub async fn continue_tasks(
        &self,
        key: &str,
    ) -> Result<Vec<BackupTask>, Box<dyn std::error::Error>> {
        unimplemented!("")
    }

    pub async fn create_new_backup_task(
        &self,
        key: &str,
        version: u32,
        prev_version: Option<u32>,
        meta: Option<&str>,
    ) -> Result<BackupTask, Box<dyn std::error::Error>> {
        unimplemented!("")
    }

    pub async fn enumerate_backup_tasks(
        &self,
        key: &str,
    ) -> Result<Vec<BackupTask>, Box<dyn std::error::Error>> {
        unimplemented!("")
    }
}
