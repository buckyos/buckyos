use tide::Request;

use serde::Deserialize;

mod backup_file_mgr;
mod backup_index;

use backup_file_mgr::*;

#[derive(Deserialize)]
struct Config {
    save_path: String,
    interface: String,
    port: u16,
}

async fn create_backup(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    backup_file_mgr.create_backup(req).await
}

async fn save_chunk(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    backup_file_mgr.save_chunk(req).await
}

async fn download_chunk(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    backup_file_mgr.download_chunk(req).await
}

async fn query_versions(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    backup_file_mgr.query_versions(req).await
}

async fn query_version_info(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    backup_file_mgr.query_version_info(req).await
}

#[async_std::main]
async fn main() -> tide::Result<()> {
    let config_path = "./config.toml";
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
    app.at("/new_chunk").post(save_chunk);
    app.at("/query_versions").get(query_versions);
    app.at("/version_info").get(query_version_info);
    app.at("/chunk").get(download_chunk);

    app.listen(format!("{}:{}", config.interface, config.port))
        .await
        .map_err(|err| {
            log::error!("listen failed: {}", err);
            err
        });

    Ok(())
}
