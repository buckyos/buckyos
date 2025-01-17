#![allow(dead_code)]
#![allow(unused)]


mod run_item;
mod kernel_mgr; // support manager kernel service (run in native, run for system)
mod service_mgr; // support manager frame service (run in docker,run for all users)
mod app_mgr; // support manager app service (run in docker,run for one user)
mod active_server;
mod run;

#[cfg(target_os = "windows")]
mod win_srv;

use std::fs::File;
use clap::{Arg, Command};
use buckyos_kit::{get_buckyos_root_dir, init_logging};
use log::*;
use simplelog::{format_description, ColorChoice, CombinedLogger, ConfigBuilder, TermLogger, TerminalMode, WriteLogger};

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new()
        .set_time_format_custom(format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"))
        .build();

    let log_path = get_buckyos_root_dir().join("logs").join("node_daemon.log");
    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            config,
            File::create(log_path).unwrap(),
        ),
    ])
        .unwrap();
}

fn main() {
    init_log_config();
    let matches = Command::new("BuckyOS Node Daemon")
        .arg(
            Arg::new("id")
                .long("node_id")
                .help("This node's id")
                .required(false),
        )
        .arg(
            Arg::new("enable_active")
                .long("enable_active")
                .help("Enable node active service")
                .action(clap::ArgAction::SetTrue)
                .required(false),
        )
        .arg(Arg::new("as_win_srv")
            .long("as_win_srv")
            .help("run as a windows service")
            .action(clap::ArgAction::SetTrue)
            .required(false))
        .get_matches();
    if matches.get_flag("as_win_srv") {
        if cfg!(windows) {
            info!("node daemon running in windows service mode");
            win_srv::service_start(matches);
        } else {
            error!("as_win_srv flag is invalid on other system");
            return;
        }
    } else {
        run::run(matches);
    }
}
