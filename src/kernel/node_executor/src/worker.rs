use buckyos_api::{ExecutorEdit, ExecutorError, ExecutorFileResult, ExecutorWriteMode};
use serde::{Deserialize, Serialize};

pub const WORKER_ARG: &str = "--executor-worker";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WorkerJob {
    ResolveDir {
        path: String,
        roots: Vec<String>,
    },
    ResolveTarget {
        path: String,
        roots: Vec<String>,
    },
    Read {
        path: String,
        roots: Vec<String>,
        offset: u64,
        length: Option<u64>,
        max_bytes: u64,
    },
    Write {
        path: String,
        roots: Vec<String>,
        mode: ExecutorWriteMode,
        expected_hash: Option<String>,
        content_length: u64,
        max_bytes: u64,
    },
    Edit {
        path: String,
        roots: Vec<String>,
        edits: Vec<ExecutorEdit>,
        expected_hash: Option<String>,
        max_bytes: u64,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkerReply {
    #[serde(default)]
    pub error: Option<ExecutorError>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub file: Option<ExecutorFileResult>,
    #[serde(default)]
    pub payload_len: u64,
    #[serde(default)]
    pub file_size: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "worker_unix.rs"]
mod unix;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix::worker_main;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn worker_main() -> ! {
    eprintln!("node executor file operations require Linux or macOS");
    std::process::exit(2)
}
