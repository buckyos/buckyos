#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::OpenOptions;
use std::process;

use buckyos_kit::init_logging;
use fs2::FileExt;

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

mod ffi_extern;

#[cfg(any(windows, target_os = "macos"))]
extern "C" {
    fn entry();
}

fn main() {
    init_logging("tray-controller", true);

    let mut file = OpenOptions::new();
    file.read(true).write(true).create(true);

    #[cfg(windows)]
    file.share_mode(0);

    let file = file
        .open(std::env::temp_dir().join("buckyos-tray-controller.lock"))
        .expect("open lock-file failed.");

    match file.try_lock_exclusive() {
        Ok(_) => {
            log::info!("buckyos tray-controller started.");

            #[cfg(any(windows, target_os = "macos"))]
            unsafe {
                entry();
            }

            #[cfg(not(any(windows, target_os = "macos")))]
            log::error!("only for windows/macos.")
        }
        Err(_) => {
            log::info!("Another tray-controller is already running.");
            process::exit(1);
        }
    }
}
