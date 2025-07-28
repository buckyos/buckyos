#![allow(dead_code)]
#![allow(unused)]


mod run_item;
mod kernel_mgr; // support manager kernel service (run in native, run for system)
mod frame_service_mgr; // support manager frame service (run in docker,run for all users)
mod app_mgr; // support manager app service (run in docker,run for one user)
mod active_server;
mod service_pkg;
mod node_daemon;
mod finder;

#[cfg(target_os = "windows")]
mod win_srv;

use clap::{Arg, Command};
use buckyos_kit::{init_logging};
use log::*;

fn main() {
    init_logging("node_daemon",true);
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
