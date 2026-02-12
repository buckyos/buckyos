#![allow(dead_code)]
#![allow(unused)]

mod active_server;
mod app_mgr; // support manager app service (run in docker,run for one user)
mod finder;
mod frame_service_mgr; // support manager frame service (run in docker,run for all users)
mod kernel_mgr; // support manager kernel service (run in native, run for system)
mod local_app_mgr;
mod node_daemon;
mod run_item;
mod service_pkg;

#[cfg(target_os = "windows")]
mod win_srv;

use buckyos_kit::{get_version, init_logging};
use clap::{Arg, Command};
use log::*;
use std::panic;

fn main() {
    init_logging("node_daemon", true);

    let matches = Command::new("BuckyOS Node Daemon")
        .version(get_version())
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
        .arg(
            Arg::new("desktop_daemon")
                .long("desktop_daemon")
                .help("Run as a desktop daemon")
                .action(clap::ArgAction::SetTrue)
                .required(false),
        )
        .arg(
            Arg::new("as_win_srv")
                .long("as_win_srv")
                .help("run as a windows service")
                .action(clap::ArgAction::SetTrue)
                .required(false),
        )
        .get_matches();
    if matches.get_flag("as_win_srv") {
        #[cfg(windows)]
        {
            info!("node daemon running in windows service mode");
            win_srv::service_start(matches);
        }
        #[cfg(not(windows))]
        {
            error!("as_win_srv flag is invalid on other system");
            return;
        }
    } else {
        node_daemon::run(matches);
    }
}
