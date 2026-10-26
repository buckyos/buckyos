#![allow(dead_code)]
#![allow(unused)]

mod active_server;
mod app_loader;
mod app_mgr; // support manager app service (run in docker,run for one user)
mod boot;
mod desktop_runtime;
mod finder;
mod frame_service_mgr; // support manager frame service (run in docker,run for all users)
mod gateway_name_provider;
mod gateway_tunnel_probe;
mod kernel_mgr; // support manager kernel service (run in native, run for system)
mod kevent_server;
mod local_app_mgr;
mod node_daemon;
mod node_executor_host;
mod paios_images;
mod region_probe;
mod run_item;
mod run_plist;
mod service_pkg;
mod sn_zone_info;
#[cfg(test)]
mod test_app_loader;
mod zone_boot_resolve;
mod wslc;

#[cfg(target_os = "windows")]
mod win_srv;

use buckyos_kit::{get_version, init_logging};
use clap::{Arg, Command};
use log::*;
use std::panic;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(node_executor::worker::WORKER_ARG) {
        node_executor::worker::worker_main();
    }
    if args.first().map(String::as_str) == Some(desktop_runtime::SERVICE_ARG) {
        std::process::exit(desktop_runtime::service_main(args[1..].to_vec()));
    }
    #[cfg(target_os = "macos")]
    if desktop_runtime::is_app_entry(&args) {
        std::process::exit(desktop_runtime::app_entry_main(&args));
    }
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
    init_logging("node_daemon", true);
    #[cfg(target_os = "macos")]
    let _instance_guard = match desktop_runtime::acquire_instance_guard() {
        Ok(guard) => guard,
        Err(err) => {
            error!("{}", err);
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    if matches.get_flag("as_win_srv") {
        #[cfg(windows)]
        {
            info!("node daemon running in windows service mode");
            if let Err(err) = win_srv::service_start(matches) {
                error!("windows service start failed: {:?}", err);
            }
        }
        #[cfg(not(windows))]
        {
            error!("as_win_srv flag is invalid on other system");
            return;
        }
    } else {
        if let Err(err) = node_daemon::run(matches) {
            error!("node daemon exited with error: {:?}", err);
        }
    }
}
