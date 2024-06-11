use std::sync::Arc;

use backup_lib::{CheckPointVersion, SimpleChunkMgrSelector, SimpleFileMgrSelector, SimpleTaskMgrSelector, TaskKey};
use rusqlite::Result;
use task_mgr::BackupTaskMgr;
use backup_task::Task;
use task_storage::FilesReadyState;
use std::env;
use std::time::Duration;
use chrono::{Local, TimeDelta, TimeZone};

mod backup_task;
mod task_mgr;
mod task_storage_sqlite;
mod task_storage;
mod restore_task;

const ETCD_BACKUP_TASK_KEY: &str = "backup.etcd";
const TASK_MGR_DB_PATH: &str = "/tmp/backup_task_mgr.db";

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


// 解析命令行，得到zone_id, server_url, etcd_servers三个字符串类型参数，以--zone_id=${} --server_url=${} --etcd_servers=${}的形式传入，以一个struct返回
struct CommandLineArgs {
    zone_id: String,
    server_url: String,
    etcd_servers: String,
}

fn parse_command_line_args() -> CommandLineArgs {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        panic!("Not enough command line arguments provided");
    }
    let mut zone_id = String::new();
    let mut server_url = String::new();
    let mut etcd_servers = String::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with("--zone_id=") {
            zone_id = arg.trim_start_matches("--zone_id=").to_string();
        } else if arg.starts_with("--server_url=") {
            server_url = arg.trim_start_matches("--server_url=").to_string();
        } else if arg.starts_with("--etcd_servers=") {
            etcd_servers = arg.trim_start_matches("--etcd_servers=").to_string();
        }
    }

    if zone_id.is_empty() || server_url.is_empty() || etcd_servers.is_empty() {
        panic!("Invalid command line arguments provided");
    }

    CommandLineArgs {
        zone_id,
        server_url,
        etcd_servers,
    }
}

#[tokio::main]
async fn main() {
    init_log_config();

    let args = parse_command_line_args();

    let chunk_mgr_selector = SimpleChunkMgrSelector::new(args.server_url.as_str());
    let file_mgr_selector = SimpleFileMgrSelector::new(args.server_url.as_str());
    let task_mgr_selector = SimpleTaskMgrSelector::new(args.server_url.as_str());

    let task_storage = task_storage_sqlite::TaskStorageSqlite::new_with_path(args.zone_id.clone(), TASK_MGR_DB_PATH).expect("create task storage failed");
    let task_storage = Arc::new(task_storage);
    let backup_task_mgr = task_mgr::BackupTaskMgr::new(
        args.zone_id.clone(),
        task_storage.clone(),
        task_storage.clone(),
        task_storage.clone(),
        Box::new(task_mgr_selector.clone()),
        Box::new(file_mgr_selector.clone()),
        Box::new(chunk_mgr_selector.clone())
    );

    backup_task_mgr.start().await.expect("Failed to start backup task manager");

    let etcd_backup_task = tokio::task::spawn(backup_etcd_process(args.etcd_servers.clone(), backup_task_mgr.clone()));
    
    let _todo_ = tokio::join!(etcd_backup_task);
}

async fn backup_etcd_process(etcd_servers: String, backup_task_mgr: BackupTaskMgr) {
    // TODO: read last backup time from a file or database
    let mut last_backup_time = Local::now() - Duration::from_secs(24 * 3600);

    loop {
        let start_time = Local.from_utc_datetime(&Local::now().date_naive().and_hms_opt(0, 0, 0).expect("Invalid time"));
        let end_time = Local.from_utc_datetime(&Local::now().date_naive().and_hms_opt(4, 0, 0).expect("Invalid time"));
        let current_time = Local::now();
        if current_time >= start_time && current_time <= end_time && current_time - last_backup_time >= TimeDelta::seconds(5 * 3600) {
            match backup_etcd_once(etcd_servers.as_str(), backup_task_mgr.clone()).await {
                Ok(_) => {
                    last_backup_time = current_time;
                },
                Err(e) => {
                    log::error!("Failed to backup etcd: {:?}", e);
                }
            }
        }

        // Sleep for 1 hour before checking again
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

async fn backup_etcd_once(etcd_servers: &str, backup_task_mgr: BackupTaskMgr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let task_key = TaskKey::from(ETCD_BACKUP_TASK_KEY);
    let backup_file = etcd_client::backup_etcd(etcd_servers).await.unwrap();
    let backup_file_path = std::path::Path::new(&backup_file);
    let dir_path = backup_file_path.parent().expect("not full path");
    let file_name = backup_file_path
        .file_name()
        .expect("no file name");


    let last_backup_task = backup_task_mgr.get_last_check_point_version(&task_key).await?;
    let last_version = last_backup_task.map_or(CheckPointVersion::from(0), |task| task.check_point_version);
    let backup_task = backup_task_mgr.backup(task_key.clone(), last_version + 1, None, None, std::path::PathBuf::from(dir_path), vec![(std::path::PathBuf::from(file_name), None)], false, 1, false).await?;
    backup_task_mgr.all_files_has_prepare_ready(backup_task.task_id()).await?;

    loop {
        let latest_task = backup_task_mgr.get_last_check_point_version(&task_key).await?;
        match latest_task {
            Some(task) => {
                if let FilesReadyState::RemoteReady = task.is_all_files_ready {
                    return Ok(());
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