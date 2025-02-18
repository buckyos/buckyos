#![windows_subsystem = "windows"]

use buckyos_kit::init_logging;

mod ffi_extern;

extern "C" {
    fn entry();
}

fn main() {
    init_logging("tray-controller");

    log::info!("buckyos tray-controller started.");
    unsafe {
        entry();
    }
}
