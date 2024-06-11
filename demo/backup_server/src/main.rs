mod backup_file_mgr;
mod backup_index;
mod main_v0;

// new version
mod main_v1;
mod task_mgr;
mod task_mgr_storage;
mod file_mgr;
mod file_mgr_storage;
mod chunk_mgr;
mod chunk_mgr_storage;

use main_v0::main_v0;
use main_v1::main_v1;

const IS_V0: bool = false;

#[tokio::main]
async fn main() {
    if IS_V0 {
        main_v0().await.unwrap();
    } else {
        main_v1().await.unwrap();
    }
}