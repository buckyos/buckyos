use std::path::PathBuf;

use crate::task_storage::{CheckPointVersion, TaskId, TaskKey};

pub trait Task {
    fn task_key(&self) -> TaskKey;
    fn task_id(&self) -> TaskId;
    fn check_point_version(&self) -> CheckPointVersion;
    fn prev_check_point_version(&self) -> Option<CheckPointVersion>;
    fn meta(&self) -> Option<String>;
    fn dir_path(&self) -> PathBuf;
    fn is_all_files_ready(&self) -> bool;
    fn is_all_files_done(&self) -> bool;
    fn file_count(&self) -> usize;
}
