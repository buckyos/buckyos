use tide::Request;

use serde::Deserialize;

use crate::backup_file_mgr::BackupFileMgr;

#[derive(Deserialize)]
struct Config {
    save_path: String,
    interface: String,
    port: u16,
}

async fn create_backup(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();
    log::info!("create_backup");
    backup_file_mgr.create_backup(req).await
}

async fn commit_backup(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();
    log::info!("commite_backup");
    backup_file_mgr.commit_backup(req).await
}

async fn save_chunk(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    log::info!("save_chunk");
    backup_file_mgr.save_chunk(req).await
}

async fn download_chunk(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    log::info!("download_chunk");
    backup_file_mgr.download_chunk(req).await
}

async fn query_versions(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    log::info!("query_versions");
    backup_file_mgr.query_versions(req).await
}

async fn query_version_info(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    log::info!("query_version_info");
    backup_file_mgr.query_version_info(req).await
}

async fn query_chunk_info(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    log::info!("query_chunk_info");
    backup_file_mgr.query_chunk_info(req).await
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

pub async fn main_v0() -> tide::Result<()> {
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

    let file_mgr = BackupFileMgr::new(config.save_path.clone()).unwrap();
    let mut app = tide::with_state(file_mgr);

    app.at("/new_backup").post(create_backup);
    app.at("/commit_backup").post(commit_backup);
    app.at("/new_chunk").post(save_chunk);
    app.at("/query_versions").get(query_versions);
    app.at("/version_info").get(query_version_info);
    app.at("/chunk_info").get(query_chunk_info);
    app.at("/chunk").get(download_chunk);

    app.listen(format!("{}:{}", config.interface, config.port))
        .await
        .map_err(|err| {
            log::error!("listen failed: {}", err);
            err
        });

    Ok(())
}
