mod downloader;
mod env;
mod error;
mod installer;
mod loader;
mod parser;
mod version_util;

use log::*;
use simplelog::*;
use std::fs;

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new().build();

    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Debug,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        // 同时将日志输出到文件
        WriteLogger::new(
            LevelFilter::Debug,
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

    let result = env.get_deps("a#1.0.1", false).await;
    info!("get_deps for a#1.0.1: {:?}", result);

    // let result = env.get_deps("a#1.0.2", false).await;
    // info!("get_deps for a#1.0.2: {:?}", result);

    //let result = env.update_lock_file();
    //info!("update_lock_file: {:?}", result);

    //let lock_file_path = env.get_work_dir().join("pkg.lock");

    //let lock_content = fs::read_to_string(lock_file_path).unwrap();
    //info!("lock_content: \n{}", lock_content);
}
