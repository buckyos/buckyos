mod downloader;
mod env;
mod error;
mod installer;
mod loader;
mod parser;
mod version_util;

use env::*;
use log::*;
use serde::{Deserialize, Serialize};
use simplelog::*;
use std::fs;

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new()
        .set_location_level(LevelFilter::Info) // 设置显示文件名和行号的日志级别
        .build();

    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        // 同时将日志输出到文件
        WriteLogger::new(
            LevelFilter::Info,
            config,
            std::fs::File::create("package_manager.log").unwrap(),
        ),
    ])
    .unwrap();
}

#[tokio::main]
async fn main() {
    init_log_config();

    // test
    let env = env::PackageEnv {
        work_dir: std::path::PathBuf::from(
            "G:\\WorkSpace\\buckyos\\demo\\package_manager\\test_env",
        ),
    };

    info!("check_lock_need_update: {:?}", env.check_lock_need_update());

    // let index_db = env.get_index(false).await.unwrap();

    // let pk_id_str = "a#1.0.1";
    // let result = env.generate_package_lock_info(&index_db, pk_id_str);

    // info!(
    //     "generate_package_lock_info for {} : {:?}",
    //     pk_id_str, result
    // );

    // info!("==>update_lock_file");

    let result = env.update_lock_file();
    info!("update_lock_file: {:?}", result);

    //info!("check_lock_need_update: {:?}", env.check_lock_need_update());

    //let result = env.get_deps("a#1.0.1", false).await;
    //info!("get_deps for a#1.0.1: {:?}", result);

    //let result = env.get_deps("a#>1.0.1", false).await;
    //info!("get_deps for a#>1.0.1: {:?}", result);

    //let result = env.update_lock_file();
    //info!("update_lock_file: {:?}", result);

    //let lock_file_path = env.get_work_dir().join("pkg.lock");

    //let lock_content = fs::read_to_string(lock_file_path).unwrap();
    //info!("lock_content: \n{}", lock_content);
}
