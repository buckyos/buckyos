use std::sync::Arc;
use serde::Deserialize;
use backup_lib::{TaskMgrHttpServer, SimpleTaskMgrSelector, FileMgrHttpServer, SimpleFileMgrSelector, ChunkMgrHttpServer, SimpleChunkMgrSelector};

use crate::{chunk_mgr::ChunkMgr, chunk_mgr_storage::ChunkStorageSqlite, file_mgr::FileMgr, file_mgr_storage::FileStorageSqlite, task_mgr::TaskMgr, task_mgr_storage::TaskStorageSqlite};
use warp::Filter;

#[derive(Deserialize)]
struct Config {
    save_path: String,
    interface: String,
    port: u16,
    access_url: String,
}

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

pub async fn main_v1() -> tide::Result<()> {
    init_log_config();

    let config_path = "./backup_server_config.toml";
    let contents = async_std::fs::read_to_string(config_path)
        .await
        .map_err(|err| {
            log::error!("read config file failed: {}", err);
            err
        })
        .unwrap();

    // 解析 TOML 字符串到 Config
    let config: Config = toml::from_str(&contents)
        .map_err(|err| {
            log::error!("parse config file failed: {}", err);
            err
        })
        .unwrap();

    let data_path = format!("{}/data", config.save_path);
    let tmp_path = format!("{}/tmp", config.save_path);
    let task_mgr_db_path = format!("{}/task_mgr.db", config.save_path);
    let file_mgr_db_path = format!("{}/file_mgr.db", config.save_path);
    let chunk_mgr_db_path = format!("{}/chunk_mgr.db", config.save_path);

    tokio::fs::create_dir_all(tmp_path.clone()).await.expect("create tmp path failed");
    tokio::fs::create_dir_all(data_path.clone()).await.expect("create tmp path failed");

    let task_storage = TaskStorageSqlite::new_with_path(task_mgr_db_path.as_str()).expect("create task storage failed");
    let file_storage = FileStorageSqlite::new_with_path(file_mgr_db_path.as_str()).expect("create file storage failed");
    let chunk_storage = ChunkStorageSqlite::new_with_path(chunk_mgr_db_path.as_str()).expect("create chunk storage failed");

    let chunk_mgr_selector = SimpleChunkMgrSelector::new(config.access_url.as_str());
    let file_mgr_selector = SimpleFileMgrSelector::new(config.access_url.as_str());
    let _todo_task_mgr_selector = SimpleTaskMgrSelector::new(config.access_url.as_str());

    let chunk_mgr = ChunkMgr::new(chunk_storage, std::path::PathBuf::from(data_path.clone()), std::path::PathBuf::from(tmp_path.clone()));
    let file_mgr = FileMgr::new(file_storage, std::sync::Arc::new(chunk_mgr_selector));
    let task_mgr = TaskMgr::new(task_storage, std::sync::Arc::new(file_mgr_selector));

    let task_mgr_http = TaskMgrHttpServer::routes(Arc::new(Box::new(task_mgr)));
    let file_mgr_http = FileMgrHttpServer::routes(Arc::new(Box::new(file_mgr)));
    let chunk_mgr_http = ChunkMgrHttpServer::routes(Arc::new(Box::new(chunk_mgr)));
    
    let log_middleware = warp::log::custom(|info| {
        log::info!("{} {} {}", info.method(), info.path(), info.status());
    });

    let routes = task_mgr_http.or(file_mgr_http).or(chunk_mgr_http).recover(|err: warp::Rejection| async move {
        log::error!("Failed to match route: {:?}", err);
        Ok::<_, warp::Rejection>(warp::reply::with_status(
            "Not Found",
            warp::http::StatusCode::NOT_FOUND,
        ))
    })
    .with(log_middleware);


    let addr = format!("{}:{}", config.interface, config.port);
    let socket_addr: std::net::SocketAddr = addr.parse().expect("Invalid address format");
    warp::serve(routes).run(socket_addr).await;

    Ok(())
}
