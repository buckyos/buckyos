use std::sync::Arc;

use backup_lib::{CheckPointVersion, SimpleChunkMgrSelector, SimpleFileMgrSelector, SimpleTaskMgrSelector, TaskKey};
use task_mgr::{BackupTaskMgr, RestoreTaskMgr};
use backup_task::Task;
use task_storage::FilesReadyState;
use tokio::fs;

mod backup_task;
mod task_mgr;
mod task_storage_sqlite;
mod task_storage;
mod restore_task;

const TASK_MGR_DB_PATH: &str = "./backup_task_mgr.db";
const TASK_MGR_SERVER_URL: &str = "http://192.168.100.137:8000";
const LOCAL_ZONE_ID: &str = "test-demo";

fn init_log_config() {
    // 创建一个日志配置对象
    let config = simplelog::ConfigBuilder::new().build();

    // 初始化日志器
    simplelog::CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        simplelog::TermLogger::new(
            log::LevelFilter::Info,
            config.clone(),
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        ),
        // 同时将日志输出到文件
        simplelog::WriteLogger::new(
            log::LevelFilter::Info,
            config,
            std::fs::File::create("backup-server.log").unwrap(),
        ),
    ])
    .unwrap();
}

#[tokio::main]
async fn main() {
    init_log_config();

    let chunk_mgr_selector = SimpleChunkMgrSelector::new(TASK_MGR_SERVER_URL);
    let file_mgr_selector = SimpleFileMgrSelector::new(TASK_MGR_SERVER_URL);
    let task_mgr_selector = SimpleTaskMgrSelector::new(TASK_MGR_SERVER_URL);

    let task_storage = task_storage_sqlite::TaskStorageSqlite::new_with_path(LOCAL_ZONE_ID.to_string(), TASK_MGR_DB_PATH).expect("create task storage failed");
    let task_storage = Arc::new(task_storage);
    let backup_task_mgr = task_mgr::BackupTaskMgr::new(
        LOCAL_ZONE_ID.to_string(),
        task_storage.clone(),
        task_storage.clone(),
        task_storage.clone(),
        Box::new(task_mgr_selector.clone()),
        Box::new(file_mgr_selector.clone()),
        Box::new(chunk_mgr_selector.clone())
    );

    backup_task_mgr.start().await.expect("Failed to start backup task manager");

    let restore_task_mgr = task_mgr::RestoreTaskMgr::new(
        LOCAL_ZONE_ID.to_string(),
        task_storage.clone(),
        Box::new(task_mgr_selector),
        Box::new(file_mgr_selector),
        Box::new(chunk_mgr_selector)
    );

    run_test(&backup_task_mgr, &restore_task_mgr).await;
}

// 在程序运行的当前目录下，创建backup目录，backup目录下有N个子目录，每个子目录以"v-$n"格式命名，$n是一个整数编号，每个目录下面有$n个文件，文件名分别命名为"f-$m"，$m是文件的整数序号，范围是[1, $n]。文件长度为($n * 8 + $m) * 1024 * 1024字节，每个字节内容为($n * $m)取最低字节
async fn make_files(version_count: usize) {
    fs::create_dir_all("./backup").await.expect("Failed to create directory");

    for i in 1..=version_count {
        let dir_name = format!("./backup/v-{}", i);
        fs::create_dir_all(&dir_name).await.expect("Failed to create directory");
        for j in 1..=i {
            let file_name = format!("f-{}", j);
            let file_size = (i * 8 + j) * 1024 * 1024;
            let file_content = (i * j) as u8;
            let file_data = vec![file_content; file_size];
            fs::write(format!("{}/{}", dir_name, file_name), file_data).await.expect("Failed to create file");
        }
    }
}

const TEST_TASK_KEY: &str = "test-task-key";

async fn backup_dir(n: usize, task_mgr: &BackupTaskMgr, task_key: TaskKey) {
    let dir_path = format!("./backup/v-{}", n);
    let mut file_list = Vec::new();
    for m in 1..=n {
        let file_name = format!("f-{}", m);
        file_list.push(std::path::PathBuf::from(file_name));
    }

    let dir_path = std::path::PathBuf::from(dir_path);
    let backup_task = task_mgr.backup(task_key.clone(), CheckPointVersion::from(n as u128), None, Some(format!("meta-v-{}", n)), dir_path, file_list.into_iter().map(|f| (f, None)).collect(), false, 1, false).await.expect("backup failed");
    task_mgr.all_files_has_prepare_ready(backup_task.task_id()).await.expect("set all-files-has-prepare-ready failed");

    loop {
        let latest_task = task_mgr.get_last_check_point_version(&task_key).await.expect("get last check point version failed");
        match latest_task {
            Some(task) => {
                if let FilesReadyState::RemoteReady = task.is_all_files_ready {
                    return ;
                } else {
                    log::info!("Task {:?} is not completed. version = {:?}, progress: {}/{}", task.task_id, task.check_point_version, task.complete_file_count, task.file_count);
                }
            },
            None => {
                assert!(false, "There should be a latest task.");
                // No latest task found, wait for some time before checking again
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

async fn restore_dir(target_version: usize, task_mgr: &RestoreTaskMgr, task_key: TaskKey) {
    let dir_path = format!("./restore/v-{}", target_version);
    fs::create_dir_all(dir_path.as_str()).await.expect("Failed to create directory for restore");
    let dir_path = std::path::PathBuf::from(dir_path);

    let last_version_task = task_mgr.get_last_check_point_version(&task_key).await.expect("get last check point version failed");
    let last_version_task = last_version_task.expect("No task found for restore");

    assert_eq!(target_version, Into::<u128>::into(last_version_task.check_point_version) as usize, "Target version not match");

    task_mgr.restore(task_key, CheckPointVersion::from(target_version as u128), dir_path.as_path()).await.expect("restore failed");

    for m in 1..=target_version {
        let file_name = format!("f-{}", m);
        let restore_file_path = format!("./restore/v-{}/{}", target_version, file_name);
        let restore_content = fs::read(restore_file_path).await.expect("Failed to read restore file");
        let backup_file_path = format!("./backup/v-{}/{}", target_version, file_name);
        let backup_content = fs::read(backup_file_path).await.expect("Failed to read original file");
        assert_eq!(&restore_content, &backup_content, "File content not match");
    }
}

async fn run_test(backup_task_mgr: &BackupTaskMgr, restore_task_mgr: &RestoreTaskMgr) {
    let task_key = TaskKey::from(format!("{}-{}", TEST_TASK_KEY, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()));

    make_files(3).await;

    backup_dir(1, &backup_task_mgr, task_key.clone()).await;

    restore_dir(1, restore_task_mgr, task_key.clone()).await;

    backup_dir(2, &backup_task_mgr, task_key.clone()).await;
    backup_dir(3, &backup_task_mgr, task_key.clone()).await;

    restore_dir(3, restore_task_mgr, task_key.clone()).await;
}